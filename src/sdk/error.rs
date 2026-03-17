use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("repository not found: {path}")]
    RepoNotFound { path: PathBuf },

    #[error("not a git repository: {path}")]
    NotGitRepo { path: PathBuf },

    #[error("failed to build call graph: {reason}")]
    IndexBuild { reason: String },

    #[error("failed to parse source file: {path}")]
    ParseError { path: String },

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    pub(crate) fn from_anyhow(err: anyhow::Error) -> Self {
        Error::Internal(format!("{err:#}"))
    }
}
