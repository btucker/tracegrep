use std::path::{Path, PathBuf};

use super::graph::Graph;

/// Builder for configuring how a [`Graph`] is loaded.
///
/// Use [`Graph::builder`] to create one.
pub struct GraphBuilder {
    repo_path: PathBuf,
    include_tests: bool,
}

impl GraphBuilder {
    pub(super) fn new(path: impl AsRef<Path>) -> Self {
        Self {
            repo_path: path.as_ref().to_path_buf(),
            include_tests: false,
        }
    }

    /// Include test files in the call graph (default: `false`).
    pub fn include_tests(mut self, include: bool) -> Self {
        self.include_tests = include;
        self
    }

    /// Build the graph, loading from cache or building from source.
    pub fn build(self) -> super::Result<Graph> {
        Graph::load_impl(self.repo_path, self.include_tests)
    }
}
