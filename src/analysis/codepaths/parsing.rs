mod javascript;
mod python;
mod rust;
mod svelte;

use std::path::Path;

use crate::analysis::is_test_file;

use super::types::{FileArtifact, Language};

pub(crate) fn extract_from_source(
    source: &str,
    relative_path: &str,
    include_tests: bool,
    language: Language,
    parser: &mut tree_sitter::Parser,
) -> Option<FileArtifact> {
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    let src = source.as_bytes();
    let module_name = module_name_from_path(relative_path, language);
    let ctx = ParseContext {
        language,
        module_name,
        file_is_test: is_test_file(relative_path),
        include_tests,
    };

    match language {
        Language::Rust => rust::extract(root, src, &ctx),
        Language::Python => python::extract(root, src, &ctx),
        Language::Svelte => svelte::extract(root, src, &ctx),
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            javascript::extract(root, src, &ctx)
        }
    }
}

#[derive(Clone)]
struct ParseContext {
    language: Language,
    module_name: String,
    file_is_test: bool,
    include_tests: bool,
}

impl ParseContext {
    fn qualified_name(&self, scopes: &[String], name: &str) -> String {
        let mut parts = Vec::new();
        if !self.module_name.is_empty() {
            parts.push(self.module_name.clone());
        }
        parts.extend(scopes.iter().cloned());
        parts.push(name.to_string());
        parts.join("::")
    }

    fn function_is_test(&self, name: &str) -> bool {
        self.file_is_test || name.starts_with("test_")
    }
}

pub(super) fn module_name_from_path(path: &str, language: Language) -> String {
    let path = Path::new(path);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let use_parent = match language {
        Language::Rust => stem == "mod" || stem == "lib" || stem == "main",
        Language::Python => stem == "__init__",
        Language::Svelte => false,
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            stem == "index"
        }
    };
    if use_parent {
        path.parent()
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        stem.into_owned()
    }
}

fn text(node: tree_sitter::Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").trim().to_string()
}

fn text_if_non_empty(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let value = text(node, src);
    (!value.is_empty()).then_some(value)
}

fn named_children(node: tree_sitter::Node) -> impl Iterator<Item = tree_sitter::Node> {
    (0..node.named_child_count()).filter_map(move |idx| node.named_child(idx))
}
