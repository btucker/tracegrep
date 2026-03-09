use super::{named_children, text, text_if_non_empty, ParseContext};
use crate::analysis::codepaths::{
    CallSite, FileArtifact, FunctionArtifact, GraphReferenceKind, ReferenceSite,
};

pub(super) fn extract(
    root: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
) -> Option<FileArtifact> {
    let mut functions = Vec::new();
    collect_functions(root, src, ctx, &mut Vec::new(), &mut functions);
    Some(FileArtifact {
        source_hash: String::new(),
        functions,
    })
}

fn collect_functions(
    node: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
    scopes: &mut Vec<String>,
    functions: &mut Vec<FunctionArtifact>,
) {
    match node.kind() {
        "class_definition" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let class_name = text(name_node, src);
            scopes.push(class_name);
            if let Some(body) = node.child_by_field_name("body") {
                for child in named_children(body) {
                    collect_functions(child, src, ctx, scopes, functions);
                }
            }
            scopes.pop();
            return;
        }
        "decorated_definition" => {
            for child in named_children(node) {
                collect_functions(child, src, ctx, scopes, functions);
            }
            return;
        }
        "function_definition" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let name = text(name_node, src);
            let is_test = ctx.function_is_test(&name);
            if !ctx.include_tests && is_test {
                return;
            }

            let line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            let qualified_name = ctx.qualified_name(scopes, &name);

            let mut call_sites = Vec::new();
            let mut reference_sites = Vec::new();
            if let Some(body) = node.child_by_field_name("body") {
                collect_calls(body, src, &mut call_sites, &mut reference_sites);
                scopes.push(name.clone());
                for child in named_children(body) {
                    collect_functions(child, src, ctx, scopes, functions);
                }
                scopes.pop();
            }

            functions.push(FunctionArtifact {
                name,
                qualified_name,
                language: ctx.language,
                is_test,
                line,
                end_line,
                call_sites,
                reference_sites,
            });
            return;
        }
        _ => {}
    }

    for child in named_children(node) {
        collect_functions(child, src, ctx, scopes, functions);
    }
}

fn collect_calls(
    node: tree_sitter::Node,
    src: &[u8],
    call_sites: &mut Vec<CallSite>,
    reference_sites: &mut Vec<ReferenceSite>,
) {
    if is_python_scope_boundary(node) {
        return;
    }

    if node.kind() == "call" {
        if let Some(function_node) = node.child_by_field_name("function") {
            let callee_name = extract_symbol_name(function_node, src);
            if !callee_name.is_empty() {
                call_sites.push(CallSite {
                    callee_name,
                    conditions: extract_conditions(node, src),
                });
            }
        }
        collect_argument_references(node, src, reference_sites);
    }

    for child in named_children(node) {
        collect_calls(child, src, call_sites, reference_sites);
    }
}

fn is_python_scope_boundary(node: tree_sitter::Node) -> bool {
    matches!(
        node.kind(),
        "function_definition" | "class_definition" | "decorated_definition" | "lambda"
    )
}

fn collect_argument_references(
    call_node: tree_sitter::Node,
    src: &[u8],
    reference_sites: &mut Vec<ReferenceSite>,
) {
    let Some(function_node) = call_node.child_by_field_name("function") else {
        return;
    };
    let Some(arguments_node) = call_node.child_by_field_name("arguments") else {
        return;
    };

    let callee_name = extract_symbol_name(function_node, src);
    for child in named_children(arguments_node) {
        collect_reference_exprs(child, src, &callee_name, reference_sites);
    }
}

fn collect_reference_exprs(
    node: tree_sitter::Node,
    src: &[u8],
    enclosing_callee: &str,
    reference_sites: &mut Vec<ReferenceSite>,
) {
    if is_python_scope_boundary(node) {
        return;
    }
    if node.kind() == "call" {
        collect_argument_references(node, src, reference_sites);
        return;
    }

    if let Some(target_name) = extract_reference_name(node, src) {
        let context = if enclosing_callee.is_empty() {
            None
        } else {
            Some(format!("passed to {enclosing_callee}"))
        };
        if !reference_sites
            .iter()
            .any(|site| site.target_name == target_name && site.context == context)
        {
            reference_sites.push(ReferenceSite {
                target_name,
                kind: GraphReferenceKind::Argument,
                context,
            });
        }
        return;
    }

    for child in named_children(node) {
        collect_reference_exprs(child, src, enclosing_callee, reference_sites);
    }
}

fn extract_conditions(call_node: tree_sitter::Node, src: &[u8]) -> Vec<String> {
    let mut conditions = Vec::new();
    let mut current = call_node.parent();

    while let Some(node) = current {
        if node.kind() == "function_definition" {
            break;
        }

        match node.kind() {
            "if_statement" | "elif_clause" => {
                if let Some(condition) = node.child_by_field_name("condition") {
                    let condition_text = text(condition, src);
                    if !condition_text.is_empty() {
                        if node.kind() == "if_statement" && is_in_else_branch(node, call_node) {
                            conditions.push(format!("!({condition_text})"));
                        } else {
                            conditions.push(condition_text);
                        }
                    }
                }
            }
            _ => {}
        }

        current = node.parent();
    }

    conditions.reverse();
    conditions
}

fn is_in_else_branch(if_node: tree_sitter::Node, descendant: tree_sitter::Node) -> bool {
    let Some(alternative) = if_node.child_by_field_name("alternative") else {
        return false;
    };
    let start = alternative.start_byte();
    let end = alternative.end_byte();
    let descendant_start = descendant.start_byte();
    descendant_start >= start && descendant_start < end
}

fn extract_reference_name(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "attribute" | "dotted_name" => text_if_non_empty(node, src),
        _ => None,
    }
}

fn extract_symbol_name(node: tree_sitter::Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" | "attribute" | "dotted_name" => text(node, src),
        _ => node.utf8_text(src).unwrap_or("").trim().to_string(),
    }
}
