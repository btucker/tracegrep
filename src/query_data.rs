use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::analysis::codepaths::CallGraph;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionIndex {
    pub by_file: HashMap<String, Vec<(usize, usize, usize)>>,
}

impl FunctionIndex {
    pub fn build(graph: &CallGraph) -> Self {
        let mut by_file: HashMap<String, Vec<(usize, usize, usize)>> = HashMap::new();
        for (i, node) in graph.nodes.iter().enumerate() {
            if node.end_line == 0 {
                continue;
            }
            let normalized = Self::normalize_path(&node.file).to_string();
            by_file
                .entry(normalized)
                .or_default()
                .push((node.line, node.end_line, i));
        }
        for intervals in by_file.values_mut() {
            intervals.sort_by_key(|&(start, _, _)| start);
        }
        Self { by_file }
    }

    pub fn lookup(&self, file: &str, line: usize) -> Option<usize> {
        let file = Self::normalize_path(file);
        let intervals = self.by_file.get(file)?;
        let pos = intervals.partition_point(|&(start, _, _)| start <= line);
        if pos == 0 {
            return None;
        }
        let (start, end, idx) = intervals[pos - 1];
        if line >= start && line <= end {
            return Some(idx);
        }
        for i in (0..pos.saturating_sub(1)).rev() {
            let (start, end, idx) = intervals[i];
            if line >= start && line <= end {
                return Some(idx);
            }
        }
        None
    }

    fn normalize_path(file: &str) -> &str {
        file.strip_prefix("./").unwrap_or(file)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryCachePayload {
    pub graph: CallGraph,
    pub function_index: FunctionIndex,
    pub backward_calls: Vec<Vec<usize>>,
    pub backward_references: Vec<Vec<usize>>,
}

impl QueryCachePayload {
    pub fn from_graph(graph: CallGraph) -> Self {
        let function_index = FunctionIndex::build(&graph);

        let mut backward_calls = vec![vec![]; graph.nodes.len()];
        for (i, edge) in graph.edges.iter().enumerate() {
            backward_calls[edge.callee].push(i);
        }

        let mut backward_references = vec![vec![]; graph.nodes.len()];
        for (i, reference) in graph.references.iter().enumerate() {
            backward_references[reference.target].push(i);
        }

        Self {
            graph,
            function_index,
            backward_calls,
            backward_references,
        }
    }
}
