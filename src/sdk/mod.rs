mod builder;
mod error;
mod graph;
mod types;

pub use builder::GraphBuilder;
pub use error::Error;
pub use graph::Graph;
pub use types::{Caller, NodeId, Reference};

/// Result type for tracegrep SDK operations.
pub type Result<T> = std::result::Result<T, Error>;
