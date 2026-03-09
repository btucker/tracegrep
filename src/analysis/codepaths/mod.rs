mod enumeration;
mod graph;
mod parsing;
mod types;

pub use graph::QueryDirection;
pub use types::{
    CallGraph, CodePath, CodePathsResult, FnDef, GraphEdge, GraphNode, GraphReference,
    GraphReferenceKind,
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::path::Path;

use enumeration::{enumerate_paths, find_roots};
use graph::{build_call_graph, build_references, build_serializable_graph};
use parsing::extract_from_source;
use types::FnCalls;

use super::is_test_file;

pub fn analyze_and_build_graph(
    repo_path: &Path,
    include_tests: bool,
) -> Result<(CodePathsResult, CallGraph)> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .context("Failed to set tree-sitter Rust language")?;

    let mut all_fn_defs: Vec<FnDef> = Vec::new();
    let mut all_fn_calls: Vec<FnCalls> = Vec::new();

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

        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(_) => continue,
        };

        extract_from_source(
            &source,
            &relative,
            include_tests,
            &mut parser,
            &mut all_fn_defs,
            &mut all_fn_calls,
        );
    }

    let graph = build_call_graph(&all_fn_defs, &all_fn_calls);
    let references = build_references(&all_fn_defs, &all_fn_calls);
    let call_graph = build_serializable_graph(&all_fn_defs, &graph, &references);
    let roots = find_roots(&all_fn_defs, &graph);
    let paths = enumerate_paths(&all_fn_defs, &graph, &roots);

    Ok((
        CodePathsResult {
            paths,
            fn_defs: all_fn_defs,
        },
        call_graph,
    ))
}
