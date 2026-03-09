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
        "class_declaration" => {
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
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                collect_named_function(node, name_node, src, ctx, scopes, functions);
                return;
            }
        }
        "method_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = property_name(name_node, src) {
                    collect_named_function_with_name(node, name, src, ctx, scopes, functions);
                    return;
                }
            }
        }
        "variable_declarator" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = match name_node.kind() {
                    "identifier" => text(name_node, src),
                    _ => String::new(),
                };
                if let Some(value_node) = node.child_by_field_name("value") {
                    if !name.is_empty() && is_function_value(value_node) {
                        collect_named_function_from_value(
                            node, value_node, name, src, ctx, scopes, functions,
                        );
                        return;
                    }
                    if !name.is_empty() && value_node.kind() == "object" {
                        scopes.push(name);
                        for child in named_children(value_node) {
                            collect_functions(child, src, ctx, scopes, functions);
                        }
                        scopes.pop();
                        return;
                    }
                }
            }
        }
        "pair" => {
            let Some(key_node) = node.child_by_field_name("key") else {
                return;
            };
            let Some(value_node) = node.child_by_field_name("value") else {
                return;
            };
            let Some(name) = property_name(key_node, src) else {
                return;
            };

            if is_function_value(value_node) {
                collect_named_function_from_value(
                    node, value_node, name, src, ctx, scopes, functions,
                );
                return;
            }
            if value_node.kind() == "object" {
                scopes.push(name);
                for child in named_children(value_node) {
                    collect_functions(child, src, ctx, scopes, functions);
                }
                scopes.pop();
                return;
            }
        }
        _ => {}
    }

    for child in named_children(node) {
        collect_functions(child, src, ctx, scopes, functions);
    }
}

fn collect_named_function(
    node: tree_sitter::Node,
    name_node: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
    scopes: &mut Vec<String>,
    functions: &mut Vec<FunctionArtifact>,
) {
    let name = text(name_node, src);
    collect_named_function_with_name(node, name, src, ctx, scopes, functions);
}

fn collect_named_function_from_value(
    definition_node: tree_sitter::Node,
    value_node: tree_sitter::Node,
    name: String,
    src: &[u8],
    ctx: &ParseContext,
    scopes: &mut Vec<String>,
    functions: &mut Vec<FunctionArtifact>,
) {
    let Some(body) = function_body(value_node) else {
        return;
    };
    let is_test = ctx.function_is_test(&name);
    if !ctx.include_tests && is_test {
        return;
    }

    let mut call_sites = Vec::new();
    let mut reference_sites = Vec::new();
    collect_calls(body, src, &mut call_sites, &mut reference_sites);

    scopes.push(name.clone());
    collect_nested_named_functions(body, src, ctx, scopes, functions);
    scopes.pop();

    functions.push(FunctionArtifact {
        name: name.clone(),
        qualified_name: ctx.qualified_name(scopes, &name),
        language: ctx.language,
        is_test,
        line: definition_node.start_position().row + 1,
        end_line: definition_node.end_position().row + 1,
        call_sites,
        reference_sites,
    });
}

fn collect_named_function_with_name(
    node: tree_sitter::Node,
    name: String,
    src: &[u8],
    ctx: &ParseContext,
    scopes: &mut Vec<String>,
    functions: &mut Vec<FunctionArtifact>,
) {
    let Some(body) = function_body(node) else {
        return;
    };
    let is_test = ctx.function_is_test(&name);
    if !ctx.include_tests && is_test {
        return;
    }

    let mut call_sites = Vec::new();
    let mut reference_sites = Vec::new();
    collect_calls(body, src, &mut call_sites, &mut reference_sites);

    scopes.push(name.clone());
    collect_nested_named_functions(body, src, ctx, scopes, functions);
    scopes.pop();

    functions.push(FunctionArtifact {
        name: name.clone(),
        qualified_name: ctx.qualified_name(scopes, &name),
        language: ctx.language,
        is_test,
        line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        call_sites,
        reference_sites,
    });
}

fn collect_nested_named_functions(
    body: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
    scopes: &mut Vec<String>,
    functions: &mut Vec<FunctionArtifact>,
) {
    for child in named_children(body) {
        collect_functions(child, src, ctx, scopes, functions);
    }
}

fn function_body(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    node.child_by_field_name("body")
}

fn is_function_value(node: tree_sitter::Node) -> bool {
    matches!(node.kind(), "function_expression" | "arrow_function")
}

fn collect_calls(
    node: tree_sitter::Node,
    src: &[u8],
    call_sites: &mut Vec<CallSite>,
    reference_sites: &mut Vec<ReferenceSite>,
) {
    if should_skip_nested_scope(node) {
        return;
    }

    if node.kind() == "call_expression" {
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

fn should_skip_nested_scope(node: tree_sitter::Node) -> bool {
    match node.kind() {
        "class_declaration" | "function_declaration" | "method_definition" => true,
        "function_expression" | "arrow_function" => is_named_value_function(node),
        _ => false,
    }
}

fn is_named_value_function(node: tree_sitter::Node) -> bool {
    matches!(
        node.parent().map(|parent| parent.kind()),
        Some("variable_declarator") | Some("pair")
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
    if should_skip_nested_scope(node) {
        return;
    }
    if node.kind() == "call_expression" {
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
        if is_function_boundary(node) {
            break;
        }

        match node.kind() {
            "if_statement" => {
                if let Some(condition) = node.child_by_field_name("condition") {
                    let condition_text = text(condition, src);
                    if !condition_text.is_empty() {
                        if is_in_else_branch(node, call_node) {
                            conditions.push(format!("!({condition_text})"));
                        } else {
                            conditions.push(condition_text);
                        }
                    }
                }
            }
            "switch_case" => {
                if let Some(value) = node.child_by_field_name("value") {
                    let case_text = text(value, src);
                    if !case_text.is_empty() {
                        conditions.push(case_text);
                    }
                }
            }
            "switch_default" => conditions.push("default".to_string()),
            _ => {}
        }

        current = node.parent();
    }

    conditions.reverse();
    conditions
}

fn is_function_boundary(node: tree_sitter::Node) -> bool {
    matches!(
        node.kind(),
        "function_declaration" | "function_expression" | "arrow_function" | "method_definition"
    )
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

fn property_name(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let value = match node.kind() {
        "identifier" | "property_identifier" | "private_property_identifier" => text(node, src),
        "string" => text(node, src).trim_matches(&['"', '\''][..]).to_string(),
        "number" => text(node, src),
        _ => return None,
    };
    (!value.is_empty()).then_some(value)
}

fn extract_reference_name(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier"
        | "property_identifier"
        | "private_property_identifier"
        | "member_expression" => text_if_non_empty(node, src),
        _ => None,
    }
}

fn extract_symbol_name(node: tree_sitter::Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier"
        | "property_identifier"
        | "private_property_identifier"
        | "member_expression" => text(node, src),
        _ => node.utf8_text(src).unwrap_or("").trim().to_string(),
    }
}
