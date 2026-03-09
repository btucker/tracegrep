use std::path::Path;

use crate::analysis::is_test_file;

use super::types::{CallSite, FnCalls, FnDef, GraphReferenceKind, ReferenceSite};

struct ParseContext<'a> {
    file: &'a str,
    module_name: &'a str,
    file_is_test: bool,
    include_tests: bool,
}

pub(super) fn extract_from_source(
    source: &str,
    relative_path: &str,
    include_tests: bool,
    parser: &mut tree_sitter::Parser,
    fn_defs: &mut Vec<FnDef>,
    fn_calls: &mut Vec<FnCalls>,
) {
    let tree = match parser.parse(source, None) {
        Some(tree) => tree,
        None => return,
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let module_name = module_name_from_path(relative_path);
    let ctx = ParseContext {
        file: relative_path,
        module_name: &module_name,
        file_is_test: is_test_file(relative_path),
        include_tests,
    };

    collect_functions(root, src, &ctx, false, fn_defs, fn_calls);
}

pub(super) fn module_name_from_path(path: &str) -> String {
    let path = Path::new(path);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    if stem == "mod" || stem == "lib" || stem == "main" {
        path.parent()
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        stem.into_owned()
    }
}

fn collect_functions(
    node: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext<'_>,
    in_test_context: bool,
    fn_defs: &mut Vec<FnDef>,
    fn_calls: &mut Vec<FnCalls>,
) {
    let is_cfg_test_module = node.kind() == "mod_item" && has_cfg_test_attr(node, src);
    if !ctx.include_tests && is_cfg_test_module {
        return;
    }

    let in_test_context = in_test_context || is_cfg_test_module;

    if node.kind() == "function_item" {
        let is_test = ctx.file_is_test || in_test_context || has_test_attr(node, src);
        if !ctx.include_tests && is_test {
            return;
        }

        if let Some(name_node) = node.child_by_field_name("name") {
            let name = name_node.utf8_text(src).unwrap_or("").to_string();
            let line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            let qualified_name = if ctx.module_name.is_empty() {
                name.clone()
            } else {
                format!("{}::{name}", ctx.module_name)
            };

            let idx = fn_defs.len();
            fn_defs.push(FnDef {
                name,
                qualified_name,
                file: ctx.file.to_string(),
                is_test,
                line,
                end_line,
            });

            let mut call_sites = Vec::new();
            let mut reference_sites = Vec::new();
            if let Some(body) = node.child_by_field_name("body") {
                collect_calls(body, src, &mut call_sites, &mut reference_sites);
            }

            fn_calls.push(FnCalls {
                caller_idx: idx,
                call_sites,
                reference_sites,
            });
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_functions(child, src, ctx, in_test_context, fn_defs, fn_calls);
        }
    }
}

fn has_cfg_test_attr(node: tree_sitter::Node, src: &[u8]) -> bool {
    preceding_attributes(node, src)
        .into_iter()
        .any(|text| text.contains("cfg(test)"))
}

fn has_test_attr(node: tree_sitter::Node, src: &[u8]) -> bool {
    preceding_attributes(node, src).into_iter().any(|text| {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let inner = compact
            .strip_prefix("#[")
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or("");
        inner == "test"
            || inner.starts_with("test(")
            || inner.ends_with("::test")
            || inner.contains("::test(")
            || inner.contains("cfg(test)")
    })
}

fn preceding_attributes<'a>(node: tree_sitter::Node<'a>, src: &'a [u8]) -> Vec<&'a str> {
    let mut attrs = Vec::new();
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() != "attribute_item" {
            break;
        }
        attrs.push(sibling.utf8_text(src).unwrap_or(""));
        prev = sibling.prev_sibling();
    }
    attrs
}

fn collect_calls(
    node: tree_sitter::Node,
    src: &[u8],
    call_sites: &mut Vec<CallSite>,
    reference_sites: &mut Vec<ReferenceSite>,
) {
    if node.kind() == "function_item" {
        return;
    }

    if node.kind() == "call_expression" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let raw = extract_callee_name(func_node, src);
            if !raw.is_empty() {
                let conditions = extract_conditions(node, src);
                call_sites.push(CallSite {
                    callee_name: raw,
                    conditions,
                });
            }
        }
        collect_argument_references(node, src, reference_sites);
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_calls(child, src, call_sites, reference_sites);
        }
    }
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

    let callee_name = extract_callee_name(function_node, src);
    for i in 0..arguments_node.named_child_count() {
        if let Some(arg) = arguments_node.named_child(i) {
            collect_reference_exprs(arg, src, &callee_name, reference_sites);
        }
    }
}

fn collect_reference_exprs(
    node: tree_sitter::Node,
    src: &[u8],
    enclosing_callee: &str,
    reference_sites: &mut Vec<ReferenceSite>,
) {
    if node.kind() == "function_item" {
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

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_reference_exprs(child, src, enclosing_callee, reference_sites);
        }
    }
}

fn extract_reference_name(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "scoped_identifier" => {
            Some(node.utf8_text(src).unwrap_or("").trim().to_string())
        }
        "generic_function" => node
            .named_child(0)
            .and_then(|child| extract_reference_name(child, src)),
        _ => None,
    }
    .filter(|name| !name.is_empty())
}

fn extract_conditions(call_node: tree_sitter::Node, src: &[u8]) -> Vec<String> {
    let mut conditions = Vec::new();
    let mut current = call_node.parent();

    while let Some(node) = current {
        if node.kind() == "function_item" {
            break;
        }

        match node.kind() {
            "if_expression" => {
                if let Some(cond_node) = node.child_by_field_name("condition") {
                    let text = cond_node.utf8_text(src).unwrap_or("").trim().to_string();
                    if !text.is_empty() {
                        let in_else = is_in_else_branch(node, call_node);
                        let cond_text = if cond_node.kind() == "let_condition" {
                            let pat = cond_node.child_by_field_name("pattern");
                            let val = cond_node.child_by_field_name("value");
                            if let (Some(pattern), Some(value)) = (pat, val) {
                                let pat_text =
                                    pattern.utf8_text(src).unwrap_or("").trim().to_string();
                                let val_text =
                                    value.utf8_text(src).unwrap_or("").trim().to_string();
                                if !pat_text.is_empty() && !val_text.is_empty() {
                                    format!("{val_text} is {pat_text}")
                                } else {
                                    text
                                }
                            } else {
                                text
                            }
                        } else {
                            text
                        };

                        if in_else {
                            conditions.push(format!("!({cond_text})"));
                        } else {
                            conditions.push(cond_text);
                        }
                    }
                }
            }
            "match_arm" => {
                if let Some(pat_node) = node.child_by_field_name("pattern") {
                    let text = pat_node.utf8_text(src).unwrap_or("").trim().to_string();
                    if !text.is_empty() && text != "_" {
                        conditions.push(text);
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
    if let Some(alt) = if_node.child_by_field_name("alternative") {
        let alt_start = alt.start_byte();
        let alt_end = alt.end_byte();
        let desc_start = descendant.start_byte();
        desc_start >= alt_start && desc_start < alt_end
    } else {
        false
    }
}

fn extract_callee_name(node: tree_sitter::Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" | "scoped_identifier" | "field_expression" => {
            node.utf8_text(src).unwrap_or("").to_string()
        }
        "generic_function" => {
            if let Some(child) = node.child_by_field_name("function") {
                extract_callee_name(child, src)
            } else if node.child_count() > 0 {
                extract_callee_name(node.child(0).unwrap(), src)
            } else {
                String::new()
            }
        }
        _ => node.utf8_text(src).unwrap_or("").to_string(),
    }
}
