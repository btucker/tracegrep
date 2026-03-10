use super::{text, text_if_non_empty, ParseContext};
use crate::analysis::codepaths::{
    CallSite, FileArtifact, FunctionArtifact, GraphReferenceKind, ReferenceSite,
};

pub(super) fn extract(
    root: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
) -> Option<FileArtifact> {
    let mut functions = Vec::new();
    collect_functions(root, src, ctx, false, &mut functions);
    Some(FileArtifact {
        source_hash: String::new(),
        functions,
    })
}

fn collect_functions(
    node: tree_sitter::Node,
    src: &[u8],
    ctx: &ParseContext,
    in_test_context: bool,
    functions: &mut Vec<FunctionArtifact>,
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
            let name = text(name_node, src);
            let line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            let qualified_name = ctx.qualified_name(&[], &name);

            let mut call_sites = Vec::new();
            let mut reference_sites = Vec::new();
            if let Some(body) = node.child_by_field_name("body") {
                collect_calls(body, src, &mut call_sites, &mut reference_sites);
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
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(u32::try_from(i).unwrap()) {
            collect_functions(child, src, ctx, in_test_context, functions);
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

    if node.kind() == "macro_invocation" {
        collect_calls_from_macro_body(node, src, call_sites, reference_sites);
        return;
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(u32::try_from(i).unwrap()) {
            collect_calls(child, src, call_sites, reference_sites);
        }
    }
}

/// Re-parses the token_tree body of a macro invocation as Rust code to find
/// call expressions that tree-sitter cannot see in the opaque token stream.
fn collect_calls_from_macro_body(
    macro_node: tree_sitter::Node,
    src: &[u8],
    call_sites: &mut Vec<CallSite>,
    reference_sites: &mut Vec<ReferenceSite>,
) {
    let token_tree = (0..macro_node.child_count())
        .filter_map(|i| macro_node.child(u32::try_from(i).unwrap()))
        .find(|child| child.kind() == "token_tree");
    let Some(tt) = token_tree else { return };
    let tt_text = tt.utf8_text(src).unwrap_or("");
    // token_tree includes the outer delimiters (braces/parens/brackets).
    // Strip them to get the inner content.
    let inner = tt_text
        .strip_prefix('{')
        .or_else(|| tt_text.strip_prefix('('))
        .or_else(|| tt_text.strip_prefix('['))
        .and_then(|s| {
            s.strip_suffix('}')
                .or_else(|| s.strip_suffix(')'))
                .or_else(|| s.strip_suffix(']'))
        })
        .unwrap_or(tt_text);

    // Wrap in a function body so tree-sitter can parse it as valid Rust.
    let synthetic = format!("fn __macro_body() {{ {inner} }}");
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return;
    }
    let Some(tree) = parser.parse(&synthetic, None) else {
        return;
    };
    let syn_src = synthetic.as_bytes();
    let root = tree.root_node();

    // Find the function body and collect calls from it.
    collect_calls_from_synthetic(root, syn_src, call_sites, reference_sites);
}

/// Walks a re-parsed synthetic tree to collect call expressions.
/// Skips condition extraction since the synthetic tree lacks the original context.
fn collect_calls_from_synthetic(
    node: tree_sitter::Node,
    src: &[u8],
    call_sites: &mut Vec<CallSite>,
    reference_sites: &mut Vec<ReferenceSite>,
) {
    if node.kind() == "function_item" {
        // Recurse into the synthetic wrapper function's body
        if let Some(body) = node.child_by_field_name("body") {
            collect_calls_from_synthetic(body, src, call_sites, reference_sites);
        }
        return;
    }

    if node.kind() == "call_expression" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let raw = extract_callee_name(func_node, src);
            if !raw.is_empty() {
                call_sites.push(CallSite {
                    callee_name: raw,
                    conditions: Vec::new(),
                });
            }
        }
    }

    // Recurse into nested macro invocations within the re-parsed body
    if node.kind() == "macro_invocation" {
        collect_calls_from_macro_body(node, src, call_sites, reference_sites);
        return;
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(u32::try_from(i).unwrap()) {
            collect_calls_from_synthetic(child, src, call_sites, reference_sites);
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
        if let Some(arg) = arguments_node.named_child(u32::try_from(i).unwrap()) {
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
        if let Some(child) = node.child(u32::try_from(i).unwrap()) {
            collect_reference_exprs(child, src, enclosing_callee, reference_sites);
        }
    }
}

fn extract_reference_name(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "scoped_identifier" => text_if_non_empty(node, src),
        "generic_function" => node
            .named_child(0)
            .and_then(|child| extract_reference_name(child, src)),
        _ => None,
    }
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
                    let condition_text = cond_node.utf8_text(src).unwrap_or("").trim().to_string();
                    if !condition_text.is_empty() {
                        let in_else = is_in_else_branch(node, call_node);
                        let cond_text = if cond_node.kind() == "let_condition" {
                            let pat = cond_node.child_by_field_name("pattern");
                            let val = cond_node.child_by_field_name("value");
                            if let (Some(pattern), Some(value)) = (pat, val) {
                                let pat_text = text(pattern, src);
                                let val_text = text(value, src);
                                if !pat_text.is_empty() && !val_text.is_empty() {
                                    format!("{val_text} is {pat_text}")
                                } else {
                                    condition_text
                                }
                            } else {
                                condition_text
                            }
                        } else {
                            condition_text
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
                    let pattern = text(pat_node, src);
                    if !pattern.is_empty() && pattern != "_" {
                        conditions.push(pattern);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::codepaths::{new_parser, Language};

    fn parse_rust_calls(source: &str) -> Vec<FunctionArtifact> {
        let mut parser = new_parser(Language::Rust).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let src = source.as_bytes();
        let ctx = ParseContext {
            language: Language::Rust,
            module_name: "test".to_string(),
            file_is_test: false,
            include_tests: false,
        };
        let artifact = extract(root, src, &ctx).unwrap();
        artifact.functions
    }

    fn callee_names(fns: &[FunctionArtifact], fn_name: &str) -> Vec<String> {
        fns.iter()
            .find(|f| f.name == fn_name)
            .map(|f| f.call_sites.iter().map(|c| c.callee_name.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn test_calls_inside_macro_select_are_found() {
        let source = r#"
fn run() {
    tokio::select! {
        result = async_op() => {
            handle_result(result);
        }
        _ = shutdown() => {
            cleanup();
        }
    }
}

fn async_op() {}
fn handle_result(_r: ()) {}
fn shutdown() {}
fn cleanup() {}
"#;
        let fns = parse_rust_calls(source);
        let callees = callee_names(&fns, "run");
        assert!(
            callees.contains(&"async_op".to_string()),
            "expected async_op in callees of run(), got: {callees:?}"
        );
        assert!(
            callees.contains(&"handle_result".to_string()),
            "expected handle_result in callees of run(), got: {callees:?}"
        );
        assert!(
            callees.contains(&"cleanup".to_string()),
            "expected cleanup in callees of run(), got: {callees:?}"
        );
    }

    #[test]
    fn test_calls_inside_assert_macro_are_found() {
        let source = r#"
fn check() {
    assert!(validate());
    assert_eq!(compute(), 42);
}

fn validate() -> bool { true }
fn compute() -> i32 { 42 }
"#;
        let fns = parse_rust_calls(source);
        let callees = callee_names(&fns, "check");
        assert!(
            callees.contains(&"validate".to_string()),
            "expected validate in callees of check(), got: {callees:?}"
        );
        assert!(
            callees.contains(&"compute".to_string()),
            "expected compute in callees of check(), got: {callees:?}"
        );
    }

    #[test]
    fn test_calls_outside_macro_still_work() {
        let source = r#"
fn main() {
    hello();
    world();
}

fn hello() {}
fn world() {}
"#;
        let fns = parse_rust_calls(source);
        let callees = callee_names(&fns, "main");
        assert!(callees.contains(&"hello".to_string()));
        assert!(callees.contains(&"world".to_string()));
    }

    #[test]
    fn test_nested_macro_calls_are_found() {
        let source = r#"
fn process() {
    println!("{}", format_data());
    vec![create_item()];
}

fn format_data() -> String { String::new() }
fn create_item() -> i32 { 0 }
"#;
        let fns = parse_rust_calls(source);
        let callees = callee_names(&fns, "process");
        assert!(
            callees.contains(&"format_data".to_string()),
            "expected format_data in callees of process(), got: {callees:?}"
        );
        assert!(
            callees.contains(&"create_item".to_string()),
            "expected create_item in callees of process(), got: {callees:?}"
        );
    }
}
