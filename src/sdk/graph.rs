use std::path::{Path, PathBuf};

use crate::graph_cache::{load_or_build_query_cache, LoadQueryResult};
use crate::query_data::QueryCachePayload;
use crate::timing::TimingCollector;

use super::builder::GraphBuilder;
use super::error::Error;

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

        // Verify git repo (supports worktrees where .git is a file)
        if !repo_path.join(".git").exists() {
            let git_check = std::process::Command::new("git")
                .args(["rev-parse", "--git-dir"])
                .current_dir(&repo_path)
                .output();
            match git_check {
                Ok(output) if output.status.success() => {}
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

    #[allow(dead_code)]
    fn payload(&self) -> &QueryCachePayload {
        &self.loaded.payload
    }
}
