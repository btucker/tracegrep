use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::graph_cache::{load_or_build_query_cache, LoadQueryResult};
use crate::query_data::QueryCachePayload;
use crate::timing::TimingCollector;

use super::builder::GraphBuilder;
use super::error::Error;
use super::types::{Caller, NodeId, Reference};

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

    // --- Query methods ---

    /// Return all callers of `node` up to `depth` levels away, using BFS.
    ///
    /// `depth = 1` returns only direct callers. `depth = 0` returns an empty
    /// list. Cycles are detected and will not cause infinite traversal.
    pub fn callers(&self, node: NodeId, depth: usize) -> Vec<Caller> {
        let payload = self.payload();
        let mut result: Vec<Caller> = Vec::new();
        let mut visited: HashSet<usize> = HashSet::new();
        // Queue entries: (node_index, current_depth)
        let mut queue: VecDeque<(usize, usize)> = VecDeque::new();

        visited.insert(node.0);
        queue.push_back((node.0, 0));

        while let Some((current_idx, current_depth)) = queue.pop_front() {
            if current_depth >= depth {
                continue;
            }
            let next_depth = current_depth + 1;
            for &edge_idx in &payload.backward_calls[current_idx] {
                let edge = &payload.graph.edges[edge_idx];
                let caller_idx = edge.caller;
                if visited.contains(&caller_idx) {
                    continue;
                }
                visited.insert(caller_idx);
                let caller_node = &payload.graph.nodes[caller_idx];
                result.push(Caller {
                    file: caller_node.file.clone(),
                    function: caller_node.name.clone(),
                    qualified_name: caller_node.qualified_name.clone(),
                    line: caller_node.line,
                    is_test: caller_node.is_test,
                    depth: next_depth,
                    conditions: edge.conditions.clone(),
                });
                queue.push_back((caller_idx, next_depth));
            }
        }

        result
    }

    /// Return the direct callees of `node` (functions called by `node`).
    ///
    /// Each callee appears at most once, even if called from multiple sites.
    pub fn callees(&self, node: NodeId) -> Vec<NodeId> {
        let payload = self.payload();
        let mut seen = HashSet::new();
        payload
            .graph
            .edges
            .iter()
            .filter(|edge| edge.caller == node.0)
            .filter(|edge| seen.insert(edge.callee))
            .map(|edge| NodeId(edge.callee))
            .collect()
    }

    /// Number of unique functions that directly call `node`.
    pub fn fan_in(&self, node: NodeId) -> usize {
        let p = self.payload();
        p.backward_calls[node.0]
            .iter()
            .map(|&edge_idx| p.graph.edges[edge_idx].caller)
            .collect::<HashSet<_>>()
            .len()
    }

    /// Number of functions `node` calls.
    pub fn fan_out(&self, node: NodeId) -> usize {
        self.callees(node).len()
    }

    /// Functions with zero callers and zero references (potential dead code).
    ///
    /// Excludes `main` and test functions. Other entry points (e.g., library
    /// exports, framework-annotated handlers) may still appear in results.
    pub fn unreachable_functions(&self) -> Vec<NodeId> {
        let p = self.payload();
        p.graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(idx, node)| {
                !node.is_test
                    && node.name != "main"
                    && p.backward_calls[*idx].is_empty()
                    && p.backward_references[*idx].is_empty()
            })
            .map(|(idx, _)| NodeId(idx))
            .collect()
    }

    /// Return all reference sites where `node` is referenced (e.g., passed as an argument).
    pub fn references(&self, node: NodeId) -> Vec<Reference> {
        let payload = self.payload();
        payload.backward_references[node.0]
            .iter()
            .map(|&ref_idx| {
                let r = &payload.graph.references[ref_idx];
                let referencer_node = &payload.graph.nodes[r.referencer];
                Reference {
                    file: referencer_node.file.clone(),
                    function: referencer_node.name.clone(),
                    qualified_name: referencer_node.qualified_name.clone(),
                    line: referencer_node.line,
                    is_test: referencer_node.is_test,
                    context: r.context.clone(),
                }
            })
            .collect()
    }
}
