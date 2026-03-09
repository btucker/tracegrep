use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub references: Vec<GraphReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: usize,
    pub name: String,
    pub qualified_name: String,
    pub file: String,
    #[serde(default)]
    pub is_test: bool,
    pub line: usize,
    #[serde(default)]
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub caller: usize,
    pub callee: usize,
    #[serde(default)]
    pub conditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GraphReferenceKind {
    Argument,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphReference {
    pub referencer: usize,
    pub target: usize,
    pub kind: GraphReferenceKind,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FnDef {
    pub name: String,
    pub qualified_name: String,
    pub file: String,
    pub is_test: bool,
    pub line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone)]
pub struct CodePath {
    pub chain: Vec<FnDef>,
}

#[derive(Debug)]
pub struct CodePathsResult {
    pub paths: Vec<CodePath>,
    pub fn_defs: Vec<FnDef>,
}

#[derive(Debug, Clone)]
pub(super) struct CallSite {
    pub callee_name: String,
    pub conditions: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ReferenceSite {
    pub target_name: String,
    pub kind: GraphReferenceKind,
    pub context: Option<String>,
}

#[derive(Debug)]
pub(super) struct FnCalls {
    pub caller_idx: usize,
    pub call_sites: Vec<CallSite>,
    pub reference_sites: Vec<ReferenceSite>,
}
