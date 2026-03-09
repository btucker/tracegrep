use std::collections::{HashMap, HashSet};

use super::graph::InternalEdge;
use super::types::{CodePath, FnDef};

pub(super) const MAX_DEPTH: usize = 10;
pub const MAX_PATHS: usize = 100_000;

pub(super) fn find_roots(
    fn_defs: &[FnDef],
    graph: &HashMap<usize, Vec<InternalEdge>>,
) -> Vec<usize> {
    let mut has_incoming: HashSet<usize> = HashSet::new();
    for edges in graph.values() {
        for edge in edges {
            has_incoming.insert(edge.callee);
        }
    }
    (0..fn_defs.len())
        .filter(|i| !has_incoming.contains(i))
        .collect()
}

pub(super) fn enumerate_paths(
    fn_defs: &[FnDef],
    graph: &HashMap<usize, Vec<InternalEdge>>,
    roots: &[usize],
) -> Vec<CodePath> {
    let mut paths = Vec::new();

    for &root in roots {
        let mut current_path: Vec<usize> = Vec::new();
        let mut visited: HashSet<usize> = HashSet::new();
        dfs(
            root,
            graph,
            fn_defs,
            &mut current_path,
            &mut visited,
            &mut paths,
        );
        if paths.len() >= MAX_PATHS {
            paths.truncate(MAX_PATHS);
            break;
        }
    }

    paths
}

fn dfs(
    node: usize,
    graph: &HashMap<usize, Vec<InternalEdge>>,
    fn_defs: &[FnDef],
    current_path: &mut Vec<usize>,
    visited: &mut HashSet<usize>,
    result: &mut Vec<CodePath>,
) {
    if result.len() >= MAX_PATHS {
        return;
    }

    current_path.push(node);
    visited.insert(node);

    let edges = graph.get(&node);
    let is_leaf = match edges {
        Some(e) => e.iter().all(|edge| visited.contains(&edge.callee)),
        None => true,
    };

    if is_leaf || current_path.len() >= MAX_DEPTH {
        let chain = current_path.iter().map(|&i| fn_defs[i].clone()).collect();
        result.push(CodePath { chain });
    } else if let Some(edges) = edges {
        for edge in edges {
            if !visited.contains(&edge.callee) {
                dfs(edge.callee, graph, fn_defs, current_path, visited, result);
            }
        }
    }

    visited.remove(&node);
    current_path.pop();
}
