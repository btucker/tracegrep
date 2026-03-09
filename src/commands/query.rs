use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::analysis::codepaths::{CallGraph, GraphReferenceKind};
use crate::graph_cache::load_or_build_graph;

struct FunctionIndex {
    by_file: HashMap<String, Vec<(usize, usize, usize)>>,
}

impl FunctionIndex {
    fn build(graph: &CallGraph) -> Self {
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

    fn normalize_path(file: &str) -> &str {
        file.strip_prefix("./").unwrap_or(file)
    }

    fn lookup(&self, file: &str, line: usize) -> Option<usize> {
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
}

#[derive(serde::Serialize)]
struct CallerInfo {
    file: String,
    function: String,
    is_test: bool,
    line: usize,
    conditions: Vec<String>,
}

struct CallerSplit {
    primary: Vec<CallerInfo>,
    test: Vec<CallerInfo>,
}

#[derive(serde::Serialize)]
struct ReferenceInfo {
    file: String,
    function: String,
    is_test: bool,
    line: usize,
    context: Option<String>,
}

struct ReferenceSplit {
    primary: Vec<ReferenceInfo>,
    test: Vec<ReferenceInfo>,
}

struct LoadedGraph {
    graph: CallGraph,
    backward_calls: Vec<Vec<usize>>,
    backward_references: Vec<Vec<usize>>,
}

pub struct QueryOptions<'a> {
    pub json_output: bool,
    pub compact: bool,
    pub repo: &'a str,
    pub depth: usize,
    pub include_tests: bool,
    pub include_test_callers: bool,
    pub pattern: &'a str,
    pub rg_args: &'a [String],
}

#[derive(Clone, Copy)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    fn from_rg_args(rg_args: &[String]) -> Self {
        let mut choice = Self::Auto;
        let mut iter = rg_args.iter();
        while let Some(arg) = iter.next() {
            if let Some(value) = arg.strip_prefix("--color=") {
                choice = Self::parse(value);
                continue;
            }

            if arg == "--color" {
                if let Some(value) = iter.next() {
                    choice = Self::parse(value);
                }
            }
        }
        choice
    }

    fn parse(value: &str) -> Self {
        match value {
            "always" | "ansi" => Self::Always,
            "never" => Self::Never,
            _ => Self::Auto,
        }
    }

    fn enabled(self) -> bool {
        match self {
            Self::Auto => std::io::stdout().is_terminal(),
            Self::Always => true,
            Self::Never => false,
        }
    }
}

struct Colors {
    enabled: bool,
}

impl Colors {
    const RESET: &str = "\x1b[0m";
    const DIM: &str = "\x1b[0m\x1b[2m";
    const PATH: &str = "\x1b[0m\x1b[35m";
    const DIM_PATH: &str = "\x1b[0m\x1b[2m\x1b[35m";
    const LINE: &str = "\x1b[0m\x1b[32m";
    const DIM_LINE: &str = "\x1b[0m\x1b[2m\x1b[32m";
    const MATCH: &str = "\x1b[0m\x1b[1m\x1b[31m";

    fn new(choice: ColorChoice) -> Self {
        Self {
            enabled: choice.enabled(),
        }
    }

    fn path(&self, value: &str) -> String {
        self.paint(value, Self::PATH)
    }

    fn line(&self, value: impl std::fmt::Display) -> String {
        self.paint(&value.to_string(), Self::LINE)
    }

    fn highlight(&self, line: &str, submatches: &[MatchSpan]) -> String {
        if !self.enabled || submatches.is_empty() {
            return line.to_string();
        }

        let mut out = String::new();
        let mut cursor = 0;
        for submatch in submatches {
            if submatch.start < cursor
                || submatch.end > line.len()
                || submatch.start >= submatch.end
                || !line.is_char_boundary(submatch.start)
                || !line.is_char_boundary(submatch.end)
            {
                return line.to_string();
            }
            out.push_str(&line[cursor..submatch.start]);
            out.push_str(Self::MATCH);
            out.push_str(&line[submatch.start..submatch.end]);
            out.push_str(Self::RESET);
            cursor = submatch.end;
        }
        out.push_str(&line[cursor..]);
        out
    }

    fn format_location(&self, file: &str, function: &str, line: usize) -> String {
        format!("{}:{}:{}", self.path(file), function, self.line(line))
    }

    fn format_caller(&self, caller: &CallerInfo) -> String {
        let mut out = format!(
            "{}:{}:{}",
            self.dim_path(&caller.file),
            self.dim(&caller.function),
            self.dim_line(caller.line)
        );
        if !caller.conditions.is_empty() {
            out.push_str(&self.dim("  (when "));
            out.push_str(&self.dim(&caller.conditions.join(" && ")));
            out.push_str(&self.dim(")"));
        }
        out
    }

    fn format_reference(&self, reference: &ReferenceInfo) -> String {
        let mut out = format!(
            "{}:{}:{}",
            self.dim_path(&reference.file),
            self.dim(&reference.function),
            self.dim_line(reference.line)
        );
        if let Some(context) = &reference.context {
            out.push_str(&self.dim("  ("));
            out.push_str(&self.dim(context));
            out.push_str(&self.dim(")"));
        }
        out
    }

    fn dim(&self, value: &str) -> String {
        self.paint(value, Self::DIM)
    }

    fn dim_path(&self, value: &str) -> String {
        self.paint(value, Self::DIM_PATH)
    }

    fn dim_line(&self, value: impl std::fmt::Display) -> String {
        self.paint(&value.to_string(), Self::DIM_LINE)
    }

    fn paint(&self, value: &str, style: &str) -> String {
        if self.enabled {
            format!("{style}{value}{}", Self::RESET)
        } else {
            value.to_string()
        }
    }
}

#[derive(Clone, Copy)]
struct MatchSpan {
    start: usize,
    end: usize,
}

fn extract_submatches(data: &serde_json::Value) -> Vec<MatchSpan> {
    data.get("submatches")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|submatch| {
            Some(MatchSpan {
                start: submatch.get("start")?.as_u64()? as usize,
                end: submatch.get("end")?.as_u64()? as usize,
            })
        })
        .collect()
}

fn collect_callers(
    graph: &CallGraph,
    backward_edges: &[Vec<usize>],
    node_idx: usize,
    depth: usize,
) -> Vec<CallerInfo> {
    let mut result = Vec::new();
    let mut current_level = vec![node_idx];
    let mut visited = HashSet::new();
    visited.insert(node_idx);

    for _ in 0..depth {
        let mut next_level = Vec::new();
        for &node_idx in &current_level {
            for &edge_idx in &backward_edges[node_idx] {
                let edge = &graph.edges[edge_idx];
                if visited.insert(edge.caller) {
                    let node = &graph.nodes[edge.caller];
                    result.push(CallerInfo {
                        file: node.file.clone(),
                        function: node.name.clone(),
                        is_test: node.is_test,
                        line: node.line,
                        conditions: edge.conditions.clone(),
                    });
                    next_level.push(edge.caller);
                }
            }
        }
        if next_level.is_empty() {
            break;
        }
        current_level = next_level;
    }

    result
}

fn split_callers(callers: Vec<CallerInfo>) -> CallerSplit {
    let (test, primary): (Vec<_>, Vec<_>) = callers.into_iter().partition(|caller| caller.is_test);
    CallerSplit { primary, test }
}

fn collect_references(
    graph: &CallGraph,
    backward_refs: &[Vec<usize>],
    node_idx: usize,
) -> Vec<ReferenceInfo> {
    let mut result = Vec::new();
    for &ref_idx in &backward_refs[node_idx] {
        let reference = &graph.references[ref_idx];
        let node = &graph.nodes[reference.referencer];
        let context = match reference.kind {
            GraphReferenceKind::Argument => reference.context.clone(),
        };
        result.push(ReferenceInfo {
            file: node.file.clone(),
            function: node.name.clone(),
            is_test: node.is_test,
            line: node.line,
            context,
        });
    }
    result
}

fn split_references(references: Vec<ReferenceInfo>) -> ReferenceSplit {
    let (test, primary): (Vec<_>, Vec<_>) = references
        .into_iter()
        .partition(|reference| reference.is_test);
    ReferenceSplit { primary, test }
}

fn load_graph(repo_path: &Path, include_tests: bool) -> anyhow::Result<LoadedGraph> {
    let graph = load_or_build_graph(repo_path, include_tests)?;

    let mut backward_calls = vec![vec![]; graph.nodes.len()];
    for (i, edge) in graph.edges.iter().enumerate() {
        backward_calls[edge.callee].push(i);
    }

    let mut backward_references = vec![vec![]; graph.nodes.len()];
    for (i, reference) in graph.references.iter().enumerate() {
        backward_references[reference.target].push(i);
    }

    Ok(LoadedGraph {
        graph,
        backward_calls,
        backward_references,
    })
}

fn summarize_hidden_test_callers(callers: &CallerSplit) -> Option<String> {
    if callers.test.is_empty() {
        None
    } else if callers.primary.is_empty() {
        Some(format!(
            "Only test callers found ({} hidden)",
            callers.test.len()
        ))
    } else {
        Some(format!(
            "{} test caller{} hidden",
            callers.test.len(),
            if callers.test.len() == 1 { "" } else { "s" }
        ))
    }
}

fn summarize_hidden_test_references(references: &ReferenceSplit) -> Option<String> {
    if references.test.is_empty() {
        None
    } else {
        Some(format!(
            "Only test references found ({} hidden)",
            references.test.len()
        ))
    }
}

fn format_compact_section(colors: &Colors, label: &str, entries: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&colors.dim("["));
    out.push_str(&colors.dim(label));
    if entries.is_empty() {
        out.push_str(&colors.dim("]"));
        return out;
    }

    out.push_str(&colors.dim(": "));
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            out.push_str(&colors.dim(", "));
        }
        out.push_str(entry);
    }
    out.push_str(&colors.dim("]"));
    out
}

fn render_compact_sections(
    colors: &Colors,
    callers: &CallerSplit,
    references: &ReferenceSplit,
    include_test_callers: bool,
) -> Vec<String> {
    let mut sections = Vec::new();

    if !callers.primary.is_empty() {
        sections.push(format_compact_section(
            colors,
            "Called via",
            &callers
                .primary
                .iter()
                .map(|caller| colors.format_caller(caller))
                .collect::<Vec<_>>(),
        ));
    }
    if include_test_callers && !callers.test.is_empty() {
        sections.push(format_compact_section(
            colors,
            "Called via tests",
            &callers
                .test
                .iter()
                .map(|caller| colors.format_caller(caller))
                .collect::<Vec<_>>(),
        ));
    } else if let Some(summary) = summarize_hidden_test_callers(callers) {
        sections.push(format_compact_section(colors, &summary, &[]));
    }

    if !references.primary.is_empty() {
        sections.push(format_compact_section(
            colors,
            "Referenced by",
            &references
                .primary
                .iter()
                .map(|reference| colors.format_reference(reference))
                .collect::<Vec<_>>(),
        ));
    } else if let Some(summary) = summarize_hidden_test_references(references) {
        sections.push(format_compact_section(colors, &summary, &[]));
    }

    sections
}

pub fn run(options: QueryOptions<'_>) -> anyhow::Result<()> {
    let repo_path = Path::new(options.repo).canonicalize()?;
    let colors = Colors::new(ColorChoice::from_rg_args(options.rg_args));
    let loaded = load_graph(&repo_path, options.include_tests)?;
    let fn_index = FunctionIndex::build(&loaded.graph);

    let mut rg_cmd = Command::new("rg");
    rg_cmd
        .arg("--json")
        .arg(options.pattern)
        .args(options.rg_args)
        .current_dir(&repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = rg_cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!(
                "rg (ripgrep) not found. Install it: https://github.com/BurntSushi/ripgrep"
            )
        } else {
            anyhow::anyhow!("Failed to run rg: {e}")
        }
    })?;

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        let line = line?;
        let parsed: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if parsed.get("type").and_then(|t| t.as_str()) != Some("match") {
            continue;
        }

        let data = match parsed.get("data") {
            Some(data) => data,
            None => continue,
        };
        let file = match data
            .get("path")
            .and_then(|path| path.get("text"))
            .and_then(|text| text.as_str())
        {
            Some(file) => file,
            None => continue,
        };
        let line_number = match data.get("line_number").and_then(|n| n.as_u64()) {
            Some(line_number) => line_number as usize,
            None => continue,
        };
        let content = data
            .get("lines")
            .and_then(|lines| lines.get("text"))
            .and_then(|text| text.as_str())
            .unwrap_or("")
            .trim_end_matches('\n');
        let highlighted_content = colors.highlight(content, &extract_submatches(data));

        let func_info = fn_index.lookup(file, line_number).map(|node_idx| {
            let node = &loaded.graph.nodes[node_idx];
            let callers = split_callers(collect_callers(
                &loaded.graph,
                &loaded.backward_calls,
                node_idx,
                options.depth,
            ));
            let references = split_references(collect_references(
                &loaded.graph,
                &loaded.backward_references,
                node_idx,
            ));
            (node, callers, references)
        });

        if options.json_output {
            let mut out = serde_json::json!({
                "file": file,
                "line": line_number,
                "content": content,
            });
            if let Some((node, callers, references)) = &func_info {
                out["function"] = serde_json::json!(node.name);
                out["qualified_name"] = serde_json::json!(node.qualified_name);
                out["is_test"] = serde_json::json!(node.is_test);
                out["callers"] = serde_json::to_value(if options.include_test_callers {
                    callers
                        .primary
                        .iter()
                        .chain(callers.test.iter())
                        .collect::<Vec<_>>()
                } else {
                    callers.primary.iter().collect::<Vec<_>>()
                })?;
                if !callers.test.is_empty() {
                    out["hidden_test_callers"] =
                        serde_json::json!(if options.include_test_callers {
                            0
                        } else {
                            callers.test.len()
                        });
                }
                out["references"] =
                    serde_json::to_value(references.primary.iter().collect::<Vec<_>>())?;
                if !references.test.is_empty() {
                    out["hidden_test_references"] = serde_json::json!(references.test.len());
                }
            }
            println!("{}", serde_json::to_string(&out)?);
            continue;
        }

        if let Some((node, callers, references)) = &func_info {
            let mut location = colors.format_location(file, &node.name, line_number);
            if options.compact {
                let compact_sections = render_compact_sections(
                    &colors,
                    callers,
                    references,
                    options.include_test_callers,
                );
                if !compact_sections.is_empty() {
                    location.push(' ');
                    location.push_str(&compact_sections.join(" "));
                }
            }
            println!("{location}");
            println!("  {highlighted_content}");
            if !options.compact {
                if !callers.primary.is_empty() {
                    println!("  {}", colors.dim("Called via:"));
                    for caller in &callers.primary {
                        println!("    {}", colors.format_caller(caller));
                    }
                }
                if options.include_test_callers && !callers.test.is_empty() {
                    println!("  {}", colors.dim("Called via tests:"));
                    for caller in &callers.test {
                        println!("    {}", colors.format_caller(caller));
                    }
                } else if let Some(summary) = summarize_hidden_test_callers(callers) {
                    println!("  {}", colors.dim(&summary));
                }
                if !references.primary.is_empty() {
                    println!("  {}", colors.dim("Referenced by:"));
                    for reference in &references.primary {
                        println!("    {}", colors.format_reference(reference));
                    }
                } else if let Some(summary) = summarize_hidden_test_references(references) {
                    println!("  {}", colors.dim(&summary));
                }
            }
            println!();
        } else {
            println!(
                "{}:{}:{}",
                colors.path(file),
                colors.line(line_number),
                highlighted_content
            );
        }
    }

    let status = child.wait()?;
    if !status.success() && status.code() != Some(1) {
        anyhow::bail!("rg exited with status {status}");
    }

    Ok(())
}
