mod enumeration;
mod graph;
mod parsing;
mod types;

pub use graph::{build_graph_from_artifacts, merge_graphs, QueryDirection};
pub(crate) use parsing::extract_from_source;
pub use types::{
    CallGraph, CallSite, CodePath, CodePathsResult, FileArtifact, FnDef, FunctionArtifact,
    GraphEdge, GraphNode, GraphReference, GraphReferenceKind, Language, ReferenceSite,
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use enumeration::{enumerate_paths, find_roots};
use graph::{
    build_call_graph, build_references, build_serializable_graph, flatten_file_artifacts,
    InternalFile,
};

use super::is_test_file;

pub fn new_parser(language: Language) -> Result<tree_sitter::Parser> {
    let mut parser = tree_sitter::Parser::new();
    let ts_language = match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::JavaScript | Language::Jsx => tree_sitter_javascript::LANGUAGE.into(),
        Language::Svelte => tree_sitter_svelte_next::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    };
    parser.set_language(&ts_language).with_context(|| {
        format!(
            "Failed to set tree-sitter {} language",
            language.display_name()
        )
    })?;
    Ok(parser)
}

pub fn language_for_path(path: &Path) -> Option<Language> {
    let extension = path.extension()?.to_str()?;
    match extension {
        "rs" => Some(Language::Rust),
        "py" => Some(Language::Python),
        "js" => Some(Language::JavaScript),
        "jsx" => Some(Language::Jsx),
        "svelte" => Some(Language::Svelte),
        "ts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        _ => None,
    }
}

pub fn collect_relevant_source_files(
    repo_path: &Path,
    include_tests: bool,
) -> BTreeMap<Language, BTreeMap<String, PathBuf>> {
    let mut files: BTreeMap<Language, BTreeMap<String, PathBuf>> = BTreeMap::new();
    let walker = WalkBuilder::new(repo_path).build();

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(language) = language_for_path(path) else {
            continue;
        };

        let relative = match path.strip_prefix(repo_path) {
            Ok(relative) => relative.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        if !include_tests && is_test_file(&relative) {
            continue;
        }
        files
            .entry(language)
            .or_default()
            .insert(relative, path.to_path_buf());
    }

    files
}

pub fn collect_relevant_files_for_language(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> BTreeMap<String, PathBuf> {
    collect_relevant_source_files(repo_path, include_tests)
        .remove(&language)
        .unwrap_or_default()
}

pub fn analyze_and_build_graph(
    repo_path: &Path,
    include_tests: bool,
) -> Result<(CodePathsResult, CallGraph)> {
    let relevant_files = collect_relevant_source_files(repo_path, include_tests);
    let mut artifacts = BTreeMap::new();

    for (language, files) in relevant_files {
        let mut parser = new_parser(language)?;
        for (relative, path) in files {
            let source = match std::fs::read_to_string(path) {
                Ok(source) => source,
                Err(_) => continue,
            };
            let Some(mut artifact) =
                extract_from_source(&source, &relative, include_tests, language, &mut parser)
            else {
                continue;
            };
            artifact.source_hash = String::new();
            artifacts.insert(relative, artifact);
        }
    }

    let files: Vec<InternalFile> = artifacts
        .iter()
        .map(|(path, artifact)| InternalFile {
            path: path.clone(),
            artifact: artifact.clone(),
        })
        .collect();
    let (fn_defs, fn_calls) = flatten_file_artifacts(&files);
    let graph = build_call_graph(&fn_defs, &fn_calls);
    let references = build_references(&fn_defs, &fn_calls);
    let call_graph = build_serializable_graph(&fn_defs, &graph, &references);
    let roots = find_roots(&fn_defs, &graph);
    let paths = enumerate_paths(&fn_defs, &graph, &roots);

    Ok((CodePathsResult { paths, fn_defs }, call_graph))
}
