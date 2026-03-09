use std::collections::{BTreeMap, HashMap};

use super::parsing::module_name_from_path;
use super::types::{CallGraph, FileArtifact, FnCalls, FnDef, GraphEdge, GraphNode, GraphReference};

pub(super) type NameIndex = HashMap<String, Vec<usize>>;

#[derive(Debug, Clone)]
pub(super) struct InternalEdge {
    pub callee: usize,
    pub conditions: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct InternalFile {
    pub path: String,
    pub artifact: FileArtifact,
}

pub(super) fn flatten_file_artifacts(files: &[InternalFile]) -> (Vec<FnDef>, Vec<FnCalls>) {
    let mut fn_defs = Vec::new();
    let mut fn_calls = Vec::new();

    for file in files {
        for function in &file.artifact.functions {
            let idx = fn_defs.len();
            fn_defs.push(FnDef {
                name: function.name.clone(),
                qualified_name: function.qualified_name.clone(),
                file: file.path.clone(),
                is_test: function.is_test,
                line: function.line,
                end_line: function.end_line,
            });
            fn_calls.push(FnCalls {
                caller_idx: idx,
                call_sites: function.call_sites.clone(),
                reference_sites: function.reference_sites.clone(),
            });
        }
    }

    (fn_defs, fn_calls)
}

pub fn build_graph_from_artifacts(files: &BTreeMap<String, FileArtifact>) -> CallGraph {
    let files: Vec<InternalFile> = files
        .iter()
        .map(|(path, artifact)| InternalFile {
            path: path.clone(),
            artifact: artifact.clone(),
        })
        .collect();
    let (fn_defs, fn_calls) = flatten_file_artifacts(&files);
    let graph = build_call_graph(&fn_defs, &fn_calls);
    let references = build_references(&fn_defs, &fn_calls);
    build_serializable_graph(&fn_defs, &graph, &references)
}

pub(super) fn build_call_graph(
    fn_defs: &[FnDef],
    fn_calls: &[FnCalls],
) -> HashMap<usize, Vec<InternalEdge>> {
    let mut name_idx: NameIndex = HashMap::new();
    for (i, def) in fn_defs.iter().enumerate() {
        name_idx.entry(def.name.clone()).or_default().push(i);
    }

    let mut graph: HashMap<usize, Vec<InternalEdge>> = HashMap::new();

    for fc in fn_calls {
        let mut edges: Vec<InternalEdge> = Vec::new();

        for site in &fc.call_sites {
            let resolved = resolve_call(&site.callee_name, fn_defs, &name_idx);
            for callee_idx in resolved {
                if let Some(existing) = edges.iter_mut().find(|e| e.callee == callee_idx) {
                    if site.conditions.is_empty() || existing.conditions.is_empty() {
                        existing.conditions.clear();
                    } else {
                        for condition in &site.conditions {
                            if !existing.conditions.contains(condition) {
                                existing.conditions.push(condition.clone());
                            }
                        }
                    }
                } else {
                    edges.push(InternalEdge {
                        callee: callee_idx,
                        conditions: site.conditions.clone(),
                    });
                }
            }
        }

        edges.sort_by_key(|e| e.callee);
        graph.insert(fc.caller_idx, edges);
    }

    graph
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryDirection {
    Forward,
    Backward,
    Both,
}

pub(super) fn build_serializable_graph(
    fn_defs: &[FnDef],
    graph: &HashMap<usize, Vec<InternalEdge>>,
    references: &[GraphReference],
) -> CallGraph {
    let nodes = fn_defs
        .iter()
        .enumerate()
        .map(|(i, def)| GraphNode {
            id: i,
            name: def.name.clone(),
            qualified_name: def.qualified_name.clone(),
            file: def.file.clone(),
            is_test: def.is_test,
            line: def.line,
            end_line: def.end_line,
        })
        .collect();

    let mut edges = Vec::new();
    for (&caller, internal_edges) in graph {
        for edge in internal_edges {
            edges.push(GraphEdge {
                caller,
                callee: edge.callee,
                conditions: edge.conditions.clone(),
            });
        }
    }
    edges.sort_by_key(|edge| (edge.caller, edge.callee));

    let mut references = references.to_vec();
    references.sort_by_key(|reference| {
        (
            reference.referencer,
            reference.target,
            reference.context.clone(),
        )
    });

    CallGraph {
        nodes,
        edges,
        references,
    }
}

pub(super) fn build_references(fn_defs: &[FnDef], fn_calls: &[FnCalls]) -> Vec<GraphReference> {
    let mut name_idx: NameIndex = HashMap::new();
    for (i, def) in fn_defs.iter().enumerate() {
        name_idx.entry(def.name.clone()).or_default().push(i);
    }

    let mut references = Vec::new();
    for fc in fn_calls {
        for site in &fc.reference_sites {
            for target in resolve_symbol(&site.target_name, fn_defs, &name_idx) {
                if target == fc.caller_idx {
                    continue;
                }
                if !references.iter().any(|existing: &GraphReference| {
                    existing.referencer == fc.caller_idx
                        && existing.target == target
                        && existing.kind == site.kind
                        && existing.context == site.context
                }) {
                    references.push(GraphReference {
                        referencer: fc.caller_idx,
                        target,
                        kind: site.kind.clone(),
                        context: site.context.clone(),
                    });
                }
            }
        }
    }

    references
}

pub(super) fn resolve_call(raw: &str, fn_defs: &[FnDef], name_idx: &NameIndex) -> Vec<usize> {
    resolve_symbol(raw, fn_defs, name_idx)
}

fn resolve_symbol(raw: &str, fn_defs: &[FnDef], name_idx: &NameIndex) -> Vec<usize> {
    let simple_name: &str;
    let qualifier: Option<&str>;

    if let Some(pos) = raw.rfind('.') {
        simple_name = &raw[pos + 1..];
        qualifier = None;
    } else if let Some(pos) = raw.rfind("::") {
        simple_name = &raw[pos + 2..];
        qualifier = Some(&raw[..pos]);
    } else {
        simple_name = raw;
        qualifier = None;
    }

    let candidates = match name_idx.get(simple_name) {
        Some(candidates) => candidates,
        None => return Vec::new(),
    };

    if let Some(qualifier) = qualifier {
        let mut matched: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|&idx| {
                let def = &fn_defs[idx];
                if def.qualified_name.starts_with(&format!("{qualifier}::")) {
                    return true;
                }
                module_name_from_path(&def.file) == qualifier
            })
            .collect();

        if !matched.is_empty() {
            matched.sort_unstable();
            matched.dedup();
            return matched;
        }
    }

    candidates.clone()
}
