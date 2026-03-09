mod enumeration;
mod graph;
mod parsing;
mod types;

pub use graph::{build_graph_from_artifacts, QueryDirection};
pub(crate) use parsing::extract_from_source;
pub use types::{
    CallGraph, CallSite, CodePath, CodePathsResult, FileArtifact, FnDef, FunctionArtifact,
    GraphEdge, GraphNode, GraphReference, GraphReferenceKind, ReferenceSite,
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::path::Path;

use enumeration::{enumerate_paths, find_roots};
use graph::{
    build_call_graph, build_references, build_serializable_graph, flatten_file_artifacts,
    InternalFile,
};

use super::is_test_file;

pub fn new_parser() -> Result<tree_sitter::Parser> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .context("Failed to set tree-sitter Rust language")?;
    Ok(parser)
}

pub fn collect_relevant_rust_files(
    repo_path: &Path,
    include_tests: bool,
) -> BTreeMap<String, std::path::PathBuf> {
    let mut files = BTreeMap::new();
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
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        let relative = match path.strip_prefix(repo_path) {
            Ok(relative) => relative.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        if !include_tests && is_test_file(&relative) {
            continue;
        }
        files.insert(relative, path.to_path_buf());
    }

    files
}

pub fn analyze_and_build_graph(
    repo_path: &Path,
    include_tests: bool,
) -> Result<(CodePathsResult, CallGraph)> {
    let mut parser = new_parser()?;
    let relevant_files = collect_relevant_rust_files(repo_path, include_tests);
    let mut artifacts = BTreeMap::new();

    for (relative, path) in relevant_files {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(_) => continue,
        };
        let Some(mut artifact) =
            extract_from_source(&source, &relative, include_tests, &mut parser)
        else {
            continue;
        };
        artifact.source_hash = String::new();
        artifacts.insert(relative, artifact);
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
