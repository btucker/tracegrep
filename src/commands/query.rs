use std::collections::HashSet;
use std::io::IsTerminal;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::analysis::codepaths::{CallGraph, GraphReferenceKind};
use crate::graph_cache::load_or_build_query_cache;
use crate::timing::TimingCollector;

#[derive(Clone, serde::Serialize)]
struct CallerInfo {
    file: String,
    function: String,
    is_test: bool,
    line: usize,
    conditions: Vec<String>,
    #[serde(skip_serializing)]
    depth: usize,
    #[serde(skip_serializing)]
    heat: usize,
}

struct CallerSplit {
    primary: Vec<CallerInfo>,
    test: Vec<CallerInfo>,
}

#[derive(Clone, serde::Serialize)]
struct ReferenceInfo {
    file: String,
    function: String,
    is_test: bool,
    line: usize,
    context: Option<String>,
    #[serde(skip_serializing)]
    heat: usize,
}

struct ReferenceSplit {
    primary: Vec<ReferenceInfo>,
    test: Vec<ReferenceInfo>,
}

pub struct QueryOptions<'a> {
    pub json_output: bool,
    pub compact: bool,
    pub repo: &'a str,
    pub search_paths: &'a [String],
    pub depth: usize,
    pub max_context: usize,
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

    fn format_location(&self, file: &str, function: &str) -> String {
        format!("{}:{}", self.path(file), function)
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

#[derive(Clone)]
struct SnippetLine {
    line_number: usize,
    content: String,
    is_match: bool,
}

struct RenderedBlock {
    file: String,
    location: String,
    match_line_number: usize,
    code_lines: Vec<SnippetLine>,
    detail_lines: Vec<String>,
}

#[derive(Clone, Copy)]
struct ContextSettings {
    before: usize,
    after: usize,
}

impl ContextSettings {
    fn from_rg_args(rg_args: &[String]) -> Self {
        let mut settings = Self {
            before: 0,
            after: 0,
        };
        let mut index = 0;
        while index < rg_args.len() {
            let arg = &rg_args[index];
            let value = if arg == "-C" || arg == "-A" || arg == "-B" {
                let parsed = rg_args
                    .get(index + 1)
                    .and_then(|value| value.parse::<usize>().ok());
                index += 1;
                parsed
            } else if let Some(value) = arg.strip_prefix("--context=") {
                value.parse::<usize>().ok()
            } else if let Some(value) = arg.strip_prefix("--after-context=") {
                value.parse::<usize>().ok()
            } else if let Some(value) = arg.strip_prefix("--before-context=") {
                value.parse::<usize>().ok()
            } else if let Some(value) = arg.strip_prefix("-C") {
                (!value.is_empty())
                    .then(|| value.parse::<usize>().ok())
                    .flatten()
            } else if let Some(value) = arg.strip_prefix("-A") {
                (!value.is_empty())
                    .then(|| value.parse::<usize>().ok())
                    .flatten()
            } else if let Some(value) = arg.strip_prefix("-B") {
                (!value.is_empty())
                    .then(|| value.parse::<usize>().ok())
                    .flatten()
            } else {
                None
            };

            if let Some(value) = value {
                if arg.starts_with("-C") || arg.starts_with("--context=") {
                    settings.before = value;
                    settings.after = value;
                } else if arg.starts_with("-A") || arg.starts_with("--after-context=") {
                    settings.after = value;
                } else if arg.starts_with("-B") || arg.starts_with("--before-context=") {
                    settings.before = value;
                }
            }

            index += 1;
        }
        settings
    }
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
    backward_refs: &[Vec<usize>],
    node_idx: usize,
    depth: usize,
) -> Vec<CallerInfo> {
    let mut result = Vec::new();
    let mut current_level = vec![node_idx];
    let mut visited = HashSet::new();
    visited.insert(node_idx);

    for current_depth in 0..depth {
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
                        depth: current_depth + 1,
                        heat: backward_edges[edge.caller].len() + backward_refs[edge.caller].len(),
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

    result.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| b.heat.cmp(&a.heat))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.function.cmp(&b.function))
    });
    result
}

fn split_callers(callers: Vec<CallerInfo>) -> CallerSplit {
    let (test, primary): (Vec<_>, Vec<_>) = callers.into_iter().partition(|caller| caller.is_test);
    CallerSplit { primary, test }
}

fn collect_references(
    graph: &CallGraph,
    backward_edges: &[Vec<usize>],
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
            heat: backward_edges[reference.referencer].len()
                + backward_refs[reference.referencer].len(),
        });
    }
    result.sort_by(|a, b| {
        b.heat
            .cmp(&a.heat)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.function.cmp(&b.function))
    });
    result
}

fn split_references(references: Vec<ReferenceInfo>) -> ReferenceSplit {
    let (test, primary): (Vec<_>, Vec<_>) = references
        .into_iter()
        .partition(|reference| reference.is_test);
    ReferenceSplit { primary, test }
}

fn truncate_context<T>(mut entries: Vec<T>, max: usize) -> (Vec<T>, usize) {
    if entries.len() <= max {
        return (entries, 0);
    }
    let hidden = entries.len() - max;
    entries.truncate(max);
    (entries, hidden)
}

fn summarize_hidden_context(label: &str, hidden: usize) -> Option<String> {
    if hidden == 0 {
        None
    } else {
        Some(format!(
            "{hidden} more {label}{} hidden",
            if hidden == 1 { "" } else { "s" }
        ))
    }
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

struct HiddenContextCounts {
    callers: usize,
    test_callers: usize,
    references: usize,
    test_references: usize,
}

fn render_compact_sections(
    colors: &Colors,
    callers: &CallerSplit,
    references: &ReferenceSplit,
    include_test_callers: bool,
    hidden: &HiddenContextCounts,
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
    } else if let Some(summary) = summarize_hidden_context("test caller", hidden.test_callers)
        .or_else(|| summarize_hidden_test_callers(callers))
    {
        sections.push(format_compact_section(colors, &summary, &[]));
    }
    if let Some(summary) = summarize_hidden_context("caller", hidden.callers) {
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
    } else if let Some(summary) = summarize_hidden_context("test reference", hidden.test_references)
        .or_else(|| summarize_hidden_test_references(references))
    {
        sections.push(format_compact_section(colors, &summary, &[]));
    }
    if let Some(summary) = summarize_hidden_context("reference", hidden.references) {
        sections.push(format_compact_section(colors, &summary, &[]));
    }

    sections
}

fn render_snippet_line(colors: &Colors, line: &SnippetLine, width: usize) -> String {
    let prefix = format!(
        "{:>width$}{}",
        line.line_number,
        if line.is_match { ":" } else { "-" }
    );
    let styled_prefix = if line.is_match {
        colors.line(prefix)
    } else {
        colors.dim_line(prefix)
    };
    let styled_content = if line.is_match {
        line.content.clone()
    } else {
        colors.dim(&line.content)
    };
    format!("{styled_prefix}{styled_content}")
}

fn flush_block(block: &mut Option<RenderedBlock>, colors: &Colors, timings: &mut TimingCollector) {
    let Some(block) = block.take() else {
        return;
    };
    let output_started_at = Instant::now();
    println!("{}", block.location);
    let width = block
        .code_lines
        .iter()
        .map(|line| line.line_number.to_string().len())
        .max()
        .unwrap_or(1);
    for line in &block.code_lines {
        println!("{}", render_snippet_line(colors, line, width));
    }
    for line in block.detail_lines {
        println!("{line}");
    }
    println!();
    timings.add("output", output_started_at.elapsed());
}

pub fn run(options: QueryOptions<'_>) -> anyhow::Result<()> {
    let repo_path = Path::new(options.repo).canonicalize()?;
    let mut timings = TimingCollector::from_env();
    let colors = Colors::new(ColorChoice::from_rg_args(options.rg_args));
    let context = ContextSettings::from_rg_args(options.rg_args);
    let loaded = load_or_build_query_cache(&repo_path, options.include_tests, &mut timings)?;
    let rg_started_at = Instant::now();

    let mut rg_cmd = Command::new("rg");
    rg_cmd
        .arg("--json")
        .args(options.rg_args)
        .arg(options.pattern)
        .args(options.search_paths)
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
    let mut pending_context_lines: Vec<SnippetLine> = Vec::new();
    let mut pending_context_file: Option<String> = None;
    let mut current_block: Option<RenderedBlock> = None;
    let mut last_rendered_line: usize = 0;
    let mut last_rendered_file: Option<String> = None;

    for line in reader.lines() {
        let line = line?;
        let parsed: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let event_type = parsed.get("type").and_then(|t| t.as_str());
        if event_type == Some("context") && !options.json_output {
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
            if current_block
                .as_ref()
                .map(|block| block.file.as_str() != file)
                .unwrap_or(false)
            {
                flush_block(&mut current_block, &colors, &mut timings);
                pending_context_lines.clear();
            }
            if pending_context_file.as_deref() != Some(file) {
                pending_context_lines.clear();
                pending_context_file = Some(file.to_string());
            }
            let context_line = SnippetLine {
                line_number,
                content: content.to_string(),
                is_match: false,
            };
            if let Some(block) = current_block.as_mut().filter(|block| block.file == file) {
                if line_number > block.match_line_number
                    && line_number <= block.match_line_number + context.after
                {
                    block.code_lines.push(context_line.clone());
                }
            }
            pending_context_lines.push(context_line);
            continue;
        }
        if event_type != Some("match") {
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

        let enrichment_started_at = Instant::now();
        let func_info = loaded
            .payload
            .function_index
            .lookup(file, line_number)
            .map(|node_idx| {
                let node = &loaded.payload.graph.nodes[node_idx];
                let callers = split_callers(collect_callers(
                    &loaded.payload.graph,
                    &loaded.payload.backward_calls,
                    &loaded.payload.backward_references,
                    node_idx,
                    options.depth,
                ));
                let references = split_references(collect_references(
                    &loaded.payload.graph,
                    &loaded.payload.backward_calls,
                    &loaded.payload.backward_references,
                    node_idx,
                ));
                (node, callers, references)
            });
        timings.add("match_enrichment", enrichment_started_at.elapsed());

        if options.json_output {
            let output_started_at = Instant::now();
            let mut out = serde_json::json!({
                "file": file,
                "line": line_number,
                "content": content,
            });
            if let Some((node, callers, references)) = &func_info {
                let (primary_callers, hidden_callers) =
                    truncate_context(callers.primary.clone(), options.max_context);
                let (test_callers, hidden_test_callers) =
                    truncate_context(callers.test.clone(), options.max_context);
                let (primary_references, hidden_references) =
                    truncate_context(references.primary.clone(), options.max_context);
                let (_, hidden_test_references) =
                    truncate_context(references.test.clone(), options.max_context);
                out["function"] = serde_json::json!(node.name);
                out["qualified_name"] = serde_json::json!(node.qualified_name);
                out["language"] = serde_json::to_value(node.language)?;
                out["is_test"] = serde_json::json!(node.is_test);
                out["callers"] = serde_json::to_value(if options.include_test_callers {
                    primary_callers
                        .iter()
                        .chain(test_callers.iter())
                        .collect::<Vec<_>>()
                } else {
                    primary_callers.iter().collect::<Vec<_>>()
                })?;
                if hidden_callers > 0 {
                    out["hidden_callers"] = serde_json::json!(hidden_callers);
                }
                if !callers.test.is_empty() {
                    out["hidden_test_callers"] =
                        serde_json::json!(if options.include_test_callers {
                            hidden_test_callers
                        } else {
                            callers.test.len()
                        });
                }
                out["references"] =
                    serde_json::to_value(primary_references.iter().collect::<Vec<_>>())?;
                if hidden_references > 0 {
                    out["hidden_references"] = serde_json::json!(hidden_references);
                }
                if !references.test.is_empty() {
                    out["hidden_test_references"] =
                        serde_json::json!(references.test.len() + hidden_test_references);
                }
            }
            println!("{}", serde_json::to_string(&out)?);
            timings.add("output", output_started_at.elapsed());
            continue;
        }

        if current_block
            .as_ref()
            .map(|block| block.file.as_str() != file)
            .unwrap_or(false)
        {
            flush_block(&mut current_block, &colors, &mut timings);
            pending_context_lines.clear();
        }
        if let Some(block) = &current_block {
            if let Some(last) = block.code_lines.last() {
                last_rendered_line = last.line_number;
                last_rendered_file = Some(block.file.clone());
            }
        }
        let mut leading_context = pending_context_lines
            .iter()
            .filter(|context_line| {
                context_line.line_number < line_number
                    && line_number - context_line.line_number <= context.before
                    && !(last_rendered_file.as_deref() == Some(file)
                        && context_line.line_number <= last_rendered_line)
            })
            .cloned()
            .collect::<Vec<_>>();
        flush_block(&mut current_block, &colors, &mut timings);
        pending_context_lines.clear();
        pending_context_file = Some(file.to_string());
        if let Some((node, callers, references)) = &func_info {
            let (primary_callers, hidden_callers) =
                truncate_context(callers.primary.clone(), options.max_context);
            let (test_callers, hidden_test_callers) =
                truncate_context(callers.test.clone(), options.max_context);
            let (primary_references, hidden_references) =
                truncate_context(references.primary.clone(), options.max_context);
            let (_, hidden_test_references) =
                truncate_context(references.test.clone(), options.max_context);
            let hidden_context = HiddenContextCounts {
                callers: hidden_callers,
                test_callers: hidden_test_callers,
                references: hidden_references,
                test_references: hidden_test_references,
            };
            let mut location = colors.format_location(file, &node.name);
            if options.compact {
                let compact_sections = render_compact_sections(
                    &colors,
                    &CallerSplit {
                        primary: primary_callers.clone(),
                        test: test_callers.clone(),
                    },
                    &ReferenceSplit {
                        primary: primary_references.clone(),
                        test: Vec::new(),
                    },
                    options.include_test_callers,
                    &hidden_context,
                );
                if !compact_sections.is_empty() {
                    location.push(' ');
                    location.push_str(&compact_sections.join(" "));
                }
            }
            leading_context.push(SnippetLine {
                line_number,
                content: highlighted_content,
                is_match: true,
            });
            let mut detail_lines = Vec::new();
            if !options.compact {
                if !primary_callers.is_empty() {
                    detail_lines.push(format!("  {}", colors.dim("Called via:")));
                    for caller in &primary_callers {
                        detail_lines.push(format!("    {}", colors.format_caller(caller)));
                    }
                }
                if let Some(summary) = summarize_hidden_context("caller", hidden_callers) {
                    detail_lines.push(format!("    {}", colors.dim(&summary)));
                }
                if options.include_test_callers && !test_callers.is_empty() {
                    detail_lines.push(format!("  {}", colors.dim("Called via tests:")));
                    for caller in &test_callers {
                        detail_lines.push(format!("    {}", colors.format_caller(caller)));
                    }
                    if let Some(summary) =
                        summarize_hidden_context("test caller", hidden_test_callers)
                    {
                        detail_lines.push(format!("    {}", colors.dim(&summary)));
                    }
                } else if let Some(summary) =
                    summarize_hidden_context("test caller", hidden_test_callers)
                        .or_else(|| summarize_hidden_test_callers(callers))
                {
                    detail_lines.push(format!("    {}", colors.dim(&summary)));
                }
                if !primary_references.is_empty() {
                    detail_lines.push(format!("  {}", colors.dim("Referenced by:")));
                    for reference in &primary_references {
                        detail_lines.push(format!("    {}", colors.format_reference(reference)));
                    }
                }
                if let Some(summary) = summarize_hidden_context("reference", hidden_references) {
                    detail_lines.push(format!("    {}", colors.dim(&summary)));
                }
                if let Some(summary) =
                    summarize_hidden_context("test reference", hidden_test_references)
                        .or_else(|| summarize_hidden_test_references(references))
                {
                    detail_lines.push(format!("    {}", colors.dim(&summary)));
                }
            }
            current_block = Some(RenderedBlock {
                file: file.to_string(),
                location,
                match_line_number: line_number,
                code_lines: leading_context,
                detail_lines,
            });
        } else {
            leading_context.push(SnippetLine {
                line_number,
                content: highlighted_content,
                is_match: true,
            });
            let location = colors.path(file);
            current_block = Some(RenderedBlock {
                file: file.to_string(),
                location,
                match_line_number: line_number,
                code_lines: leading_context,
                detail_lines: Vec::new(),
            });
        }
    }

    flush_block(&mut current_block, &colors, &mut timings);
    let status = child.wait()?;
    timings.add("rg_run", rg_started_at.elapsed());
    if !status.success() && status.code() != Some(1) {
        anyhow::bail!("rg exited with status {status}");
    }

    timings.print("query");
    Ok(())
}
