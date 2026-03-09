use super::{javascript, named_children, ParseContext};
use crate::analysis::codepaths::{
    self, CallSite, FileArtifact, FunctionArtifact, GraphReferenceKind, Language, ReferenceSite,
};

pub(super) fn extract(
    root: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
) -> Option<FileArtifact> {
    let mut functions = Vec::new();
    collect_script_functions(root, src, ctx, &mut functions);
    if let Some(template) = build_template_artifact(root, src, ctx) {
        functions.push(template);
    }

    Some(FileArtifact {
        source_hash: String::new(),
        functions,
    })
}

fn collect_script_functions(
    node: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
    functions: &mut Vec<FunctionArtifact>,
) {
    if node.kind() == "script_element" {
        for child in named_children(node) {
            if child.kind() != "raw_text" {
                continue;
            }
            let script = child.utf8_text(src).unwrap_or("").trim();
            if script.is_empty() {
                continue;
            }
            let Some(mut parser) = codepaths::new_parser(Language::TypeScript).ok() else {
                continue;
            };
            let Some(tree) = parser.parse(script, None) else {
                continue;
            };
            let Some(file) = javascript::extract(tree.root_node(), script.as_bytes(), ctx) else {
                continue;
            };
            let line_offset = child.start_position().row;
            functions.extend(file.functions.into_iter().map(|mut function| {
                function.line += line_offset;
                function.end_line += line_offset;
                function
            }));
        }
        return;
    }

    for child in named_children(node) {
        collect_script_functions(child, src, ctx, functions);
    }
}

fn build_template_artifact(
    root: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
) -> Option<FunctionArtifact> {
    let mut call_sites = Vec::new();
    let mut reference_sites = Vec::new();
    let mut first_line = None;
    let mut last_line = 0;

    collect_template_expressions(
        root,
        src,
        ctx,
        &mut call_sites,
        &mut reference_sites,
        &mut first_line,
        &mut last_line,
    );

    let line = first_line?;
    if call_sites.is_empty() && reference_sites.is_empty() {
        return None;
    }

    Some(FunctionArtifact {
        name: "template".to_string(),
        qualified_name: ctx.qualified_name(&[], "template"),
        language: ctx.language,
        is_test: ctx.file_is_test,
        line,
        end_line: last_line.max(line),
        call_sites,
        reference_sites,
    })
}

fn collect_template_expressions(
    node: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
    call_sites: &mut Vec<CallSite>,
    reference_sites: &mut Vec<ReferenceSite>,
    first_line: &mut Option<usize>,
    last_line: &mut usize,
) {
    if node.kind() == "script_element" || node.kind() == "style_element" {
        return;
    }

    if node.kind() == "svelte_raw_text" {
        let expression = node.utf8_text(src).unwrap_or("").trim();
        if !expression.is_empty() {
            let line = node.start_position().row + 1;
            *first_line = Some(first_line.map_or(line, |current| current.min(line)));
            *last_line = (*last_line).max(node.end_position().row + 1);
            extract_template_expression(expression, ctx, call_sites, reference_sites);
        }
    }

    for child in named_children(node) {
        collect_template_expressions(
            child,
            src,
            ctx,
            call_sites,
            reference_sites,
            first_line,
            last_line,
        );
    }
}

fn extract_template_expression(
    expression: &str,
    ctx: &ParseContext,
    call_sites: &mut Vec<CallSite>,
    reference_sites: &mut Vec<ReferenceSite>,
) {
    let wrapped = format!("function __tracegrep_template__() {{ {expression}; }}");
    let Some(mut parser) = codepaths::new_parser(Language::TypeScript).ok() else {
        return;
    };
    let Some(tree) = parser.parse(&wrapped, None) else {
        return;
    };
    let Some(file) = javascript::extract(tree.root_node(), wrapped.as_bytes(), ctx) else {
        return;
    };

    if let Some(template_fn) = file
        .functions
        .into_iter()
        .find(|function| function.name == "__tracegrep_template__")
    {
        for call_site in template_fn.call_sites {
            if !call_sites.iter().any(|existing| existing == &call_site) {
                call_sites.push(call_site);
            }
        }
        for reference_site in template_fn.reference_sites {
            if !reference_sites
                .iter()
                .any(|existing| existing == &reference_site)
            {
                reference_sites.push(reference_site);
            }
        }
        return;
    }

    if let Some(target_name) = simple_template_reference(expression) {
        let reference_site = ReferenceSite {
            target_name,
            kind: GraphReferenceKind::Argument,
            context: Some("used in template".to_string()),
        };
        if !reference_sites
            .iter()
            .any(|existing| existing == &reference_site)
        {
            reference_sites.push(reference_site);
        }
    }
}

fn simple_template_reference(expression: &str) -> Option<String> {
    let trimmed = expression.trim();
    if trimmed.is_empty() || trimmed.contains('(') || trimmed.contains('=') || trimmed.contains(' ')
    {
        return None;
    }
    let value = trimmed
        .trim_matches(|c: char| c == '{' || c == '}' || c == ';')
        .to_string();
    (!value.is_empty()).then_some(value)
}
