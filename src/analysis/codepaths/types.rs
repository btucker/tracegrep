use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    Jsx,
    Svelte,
    TypeScript,
    Tsx,
}

impl Language {
    pub const ALL: [Language; 7] = [
        Language::Rust,
        Language::Python,
        Language::JavaScript,
        Language::Jsx,
        Language::Svelte,
        Language::TypeScript,
        Language::Tsx,
    ];

    pub fn cache_key(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::Jsx => "jsx",
            Self::Svelte => "svelte",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::JavaScript => "JavaScript",
            Self::Jsx => "JSX",
            Self::Svelte => "Svelte",
            Self::TypeScript => "TypeScript",
            Self::Tsx => "TSX",
        }
    }
}

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
    pub language: Language,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
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
    pub language: Language,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallSite {
    pub callee_name: String,
    #[serde(default)]
    pub conditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceSite {
    pub target_name: String,
    pub kind: GraphReferenceKind,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionArtifact {
    pub name: String,
    pub qualified_name: String,
    pub language: Language,
    pub is_test: bool,
    pub line: usize,
    pub end_line: usize,
    #[serde(default)]
    pub call_sites: Vec<CallSite>,
    #[serde(default)]
    pub reference_sites: Vec<ReferenceSite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileArtifact {
    pub source_hash: String,
    #[serde(default)]
    pub functions: Vec<FunctionArtifact>,
}

#[derive(Debug, Clone)]
pub(super) struct FnCalls {
    pub caller_idx: usize,
    pub call_sites: Vec<CallSite>,
    pub reference_sites: Vec<ReferenceSite>,
}
