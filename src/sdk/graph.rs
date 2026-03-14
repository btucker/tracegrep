use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::graph_cache::{load_or_build_query_cache, LoadQueryResult};
use crate::query_data::QueryCachePayload;
use crate::timing::TimingCollector;

use super::builder::GraphBuilder;
use super::error::Error;
use super::types::NodeId;

/// A loaded call graph with pre-built indexes for fast querying.
pub struct Graph {
    #[allow(dead_code)]
    repo_path: PathBuf,
    loaded: LoadQueryResult,
}

impl Graph {
    /// Load the call graph for a repository, using defaults.
    ///
    /// Tests are excluded by default. Use [`Graph::builder`] to include them.
    pub fn load(path: impl AsRef<Path>) -> super::Result<Self> {
        Self::load_impl(path.as_ref().to_path_buf(), false)
    }

    /// Create a builder for fine-grained control over graph loading.
    pub fn builder(path: impl AsRef<Path>) -> GraphBuilder {
        GraphBuilder::new(path)
    }

    pub(super) fn load_impl(repo_path: PathBuf, include_tests: bool) -> super::Result<Self> {
        let repo_path = repo_path.canonicalize().map_err(|_| Error::RepoNotFound {
            path: repo_path.clone(),
        })?;

        // Verify this is a git repo root, not a subdirectory.
        // .git can be a directory (normal repo) or a file (worktree).
        if !repo_path.join(".git").exists() {
            // No .git at this path — check if it's inside a repo (subdirectory)
            let git_check = std::process::Command::new("git")
                .args(["rev-parse", "--show-toplevel"])
                .current_dir(&repo_path)
                .output();
            match git_check {
                Ok(output) if output.status.success() => {
                    let toplevel = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
                    if toplevel.canonicalize().ok().as_ref() != Some(&repo_path) {
                        return Err(Error::NotGitRepo { path: repo_path });
                    }
                }
                _ => return Err(Error::NotGitRepo { path: repo_path }),
            }
        }

        let mut timings = TimingCollector::disabled();
        let loaded = load_or_build_query_cache(&repo_path, include_tests, &mut timings)
            .map_err(Error::from_anyhow)?;

        Ok(Self { repo_path, loaded })
    }

    /// Total number of function nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.loaded.payload.graph.nodes.len()
    }

    /// Access the underlying `CallGraph` data (escape hatch for advanced use).
    #[doc(hidden)]
    pub fn raw(&self) -> &crate::analysis::codepaths::CallGraph {
        &self.loaded.payload.graph
    }

    fn payload(&self) -> &QueryCachePayload {
        &self.loaded.payload
    }

    // --- Lookups ---

    /// Find the function at a specific file path and line number.
    pub fn function_at(&self, file: &str, line: usize) -> Option<NodeId> {
        self.payload().function_index.lookup(file, line).map(NodeId)
    }

    /// Find all functions matching a simple name (e.g., `"new"`).
    pub fn functions_by_name(&self, name: &str) -> Vec<NodeId> {
        self.payload()
            .graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.name == name)
            .map(|(idx, _)| NodeId(idx))
            .collect()
    }

    /// Find all functions matching a qualified name (e.g., `"MyStruct::new"`).
    pub fn functions_by_qualified_name(&self, qualified_name: &str) -> Vec<NodeId> {
        self.payload()
            .graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.qualified_name == qualified_name)
            .map(|(idx, _)| NodeId(idx))
            .collect()
    }

    /// All function nodes in the graph.
    pub fn functions(&self) -> Vec<NodeId> {
        (0..self.payload().graph.nodes.len()).map(NodeId).collect()
    }

    // --- Accessors ---
    //
    // All accessor methods below require that `node` was obtained from this
    // same `Graph` instance. Using a `NodeId` from a different `Graph` will
    // panic with an index-out-of-bounds error.

    /// Simple function name.
    ///
    /// # Panics
    /// Panics if `node` did not originate from this `Graph`.
    pub fn function_name(&self, node: NodeId) -> &str {
        &self.payload().graph.nodes[node.0].name
    }

    /// Fully qualified name (e.g., `MyStruct::new`).
    pub fn function_qualified_name(&self, node: NodeId) -> &str {
        &self.payload().graph.nodes[node.0].qualified_name
    }

    /// File path relative to repo root.
    pub fn function_file(&self, node: NodeId) -> &str {
        &self.payload().graph.nodes[node.0].file
    }

    /// Line number where the function is defined.
    pub fn function_line(&self, node: NodeId) -> usize {
        self.payload().graph.nodes[node.0].line
    }

    /// End line of the function definition.
    pub fn function_end_line(&self, node: NodeId) -> usize {
        self.payload().graph.nodes[node.0].end_line
    }

    /// Whether this function is in test code.
    pub fn function_is_test(&self, node: NodeId) -> bool {
        self.payload().graph.nodes[node.0].is_test
    }

    /// Files that changed since the last index build.
    /// Empty if the cache was fully reused.
    pub fn changed_files(&self) -> &[String] {
        &self.loaded.outcome.changed_files
    }

    /// Functions defined in files that changed since the last index build.
    /// This is the core loop for incremental analysis: only check changed functions.
    pub fn changed_functions(&self) -> Vec<NodeId> {
        let changed: HashSet<&str> = self
            .loaded
            .outcome
            .changed_files
            .iter()
            .map(|s| s.as_str())
            .collect();

        if changed.is_empty() {
            return Vec::new();
        }

        self.payload()
            .graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| changed.contains(node.file.as_str()))
            .map(|(idx, _)| NodeId(idx))
            .collect()
    }
}
