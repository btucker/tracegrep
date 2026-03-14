/// Opaque identifier for a function node in the call graph.
///
/// Obtained from `Graph::function_at` or iteration methods.
/// Pass to query methods like `Graph::callers`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub(super) usize);

/// A function that calls the queried function.
#[derive(Debug, Clone)]
pub struct Caller {
    pub file: String,
    pub function: String,
    pub qualified_name: String,
    pub line: usize,
    pub is_test: bool,
    /// How many call levels away from the target (1 = direct caller).
    pub depth: usize,
    pub conditions: Vec<String>,
}

/// A function that references the queried function (e.g., passes it as an argument).
#[derive(Debug, Clone)]
pub struct Reference {
    pub file: String,
    pub function: String,
    pub qualified_name: String,
    pub line: usize,
    pub is_test: bool,
    pub context: Option<String>,
}
