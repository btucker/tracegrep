mod error;
mod types;

pub use error::Error;
pub use types::{Caller, NodeId, Reference};

/// Result type for tracegrep SDK operations.
pub type Result<T> = std::result::Result<T, Error>;
