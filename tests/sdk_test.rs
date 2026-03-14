use std::process::Command;
use tracegrep::sdk::{Caller, Error, Graph, NodeId, Reference};

fn init_test_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    for (path, contents) in files {
        let full_path = dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, contents).unwrap();
    }
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    git(&["init"]);
    git(&["config", "user.email", "test@test.com"]);
    git(&["config", "user.name", "Test"]);
    git(&["add", "."]);
    git(&["commit", "-m", "initial"]);
    let repo_path = dir.path().canonicalize().unwrap();
    (dir, repo_path)
}

#[test]
fn graph_load_opens_repo() {
    let (_dir, repo_path) =
        init_test_repo(&[("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n")]);
    let graph = Graph::load(&repo_path).unwrap();
    assert!(graph.node_count() >= 2, "expected at least main and hello");
}

#[test]
fn graph_load_returns_error_for_nonexistent_path() {
    let result = Graph::load("/nonexistent/path/to/repo");
    assert!(result.is_err());
}

#[test]
fn graph_builder_with_tests() {
    let (_dir, repo_path) = init_test_repo(&[
        ("src/main.rs", "fn main() {}\n"),
        ("tests/it.rs", "#[test]\nfn test_it() {}\n"),
    ]);

    let without = Graph::load(&repo_path).unwrap();
    let with = Graph::builder(&repo_path)
        .include_tests(true)
        .build()
        .unwrap();

    assert!(
        with.node_count() > without.node_count(),
        "with_tests={} should be > without={}",
        with.node_count(),
        without.node_count()
    );
    let test_nodes: Vec<_> = with
        .functions()
        .into_iter()
        .filter(|n| with.function_is_test(*n))
        .collect();
    assert!(
        !test_nodes.is_empty(),
        "should have at least one test function"
    );
}

#[test]
fn graph_function_at_finds_by_file_and_line() {
    let (_dir, repo_path) =
        init_test_repo(&[("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n")]);
    let graph = Graph::load(&repo_path).unwrap();

    let node = graph.function_at("src/main.rs", 1);
    assert!(node.is_some(), "should find main at line 1");
    assert_eq!(graph.function_name(node.unwrap()), "main");
}

#[test]
fn graph_functions_by_name_finds_matches() {
    let (_dir, repo_path) =
        init_test_repo(&[("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n")]);
    let graph = Graph::load(&repo_path).unwrap();

    let nodes = graph.functions_by_name("hello");
    assert_eq!(nodes.len(), 1);
    assert_eq!(graph.function_name(nodes[0]), "hello");
}

#[test]
fn graph_function_accessors() {
    let (_dir, repo_path) = init_test_repo(&[(
        "src/main.rs",
        "fn main() {\n    hello();\n}\nfn hello() {}\n",
    )]);
    let graph = Graph::load(&repo_path).unwrap();

    let nodes = graph.functions_by_name("main");
    let node = nodes[0];
    assert_eq!(graph.function_name(node), "main");
    assert_eq!(graph.function_file(node), "src/main.rs");
    assert_eq!(graph.function_line(node), 1);
    assert!(!graph.function_is_test(node));
}

#[test]
fn error_is_send_sync_and_displays() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Error>();

    let err = Error::RepoNotFound {
        path: "/nonexistent".into(),
    };
    let msg = err.to_string();
    assert!(msg.contains("/nonexistent"), "msg: {msg}");
}

#[test]
fn node_id_is_copy_and_debug() {
    fn assert_copy_debug<T: Copy + std::fmt::Debug>() {}
    assert_copy_debug::<NodeId>();
}

#[test]
fn caller_and_reference_have_expected_fields() {
    let caller = Caller {
        file: "src/main.rs".into(),
        function: "main".into(),
        qualified_name: "main".into(),
        line: 1,
        is_test: false,
        depth: 1,
        conditions: vec!["method == POST".into()],
    };
    assert_eq!(caller.file, "src/main.rs");
    assert_eq!(caller.depth, 1);

    let reference = Reference {
        file: "src/main.rs".into(),
        function: "register".into(),
        qualified_name: "register".into(),
        line: 5,
        is_test: false,
        context: Some("passed to register_handler".into()),
    };
    assert!(reference.context.is_some());
}

#[test]
fn caller_and_reference_support_hash_dedup() {
    use std::collections::HashSet;

    let caller = Caller {
        file: "src/main.rs".into(),
        function: "main".into(),
        qualified_name: "main".into(),
        line: 1,
        is_test: false,
        depth: 1,
        conditions: vec![],
    };
    let mut set = HashSet::new();
    set.insert(caller.clone());
    set.insert(caller);
    assert_eq!(set.len(), 1, "identical Callers should deduplicate in HashSet");

    let reference = Reference {
        file: "src/lib.rs".into(),
        function: "init".into(),
        qualified_name: "init".into(),
        line: 10,
        is_test: false,
        context: None,
    };
    let mut set = HashSet::new();
    set.insert(reference.clone());
    set.insert(reference);
    assert_eq!(set.len(), 1, "identical References should deduplicate in HashSet");
}

#[test]
fn graph_load_rejects_subdirectory_path() {
    let (_dir, repo_path) = init_test_repo(&[("src/main.rs", "fn main() {}\n")]);
    let result = Graph::load(repo_path.join("src"));
    assert!(
        result.is_err(),
        "loading a subdirectory should fail, not silently produce a wrong graph"
    );
}

#[test]
fn graph_callers_returns_direct_callers() {
    let (_dir, repo_path) = init_test_repo(&[
        ("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n"),
    ]);
    let graph = Graph::load(&repo_path).unwrap();

    let hello = graph.functions_by_name("hello");
    assert_eq!(hello.len(), 1);

    let callers = graph.callers(hello[0], 1);
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].function, "main");
    assert_eq!(callers[0].depth, 1);
}

#[test]
fn graph_callers_respects_depth() {
    let (_dir, repo_path) = init_test_repo(&[(
        "src/main.rs",
        "fn a() { b(); }\nfn b() { c(); }\nfn c() {}\n",
    )]);
    let graph = Graph::load(&repo_path).unwrap();

    let c = graph.functions_by_name("c");
    assert_eq!(c.len(), 1);

    let depth1 = graph.callers(c[0], 1);
    assert_eq!(depth1.len(), 1, "depth 1 should find only b");
    assert_eq!(depth1[0].function, "b");

    let depth2 = graph.callers(c[0], 2);
    assert_eq!(depth2.len(), 2, "depth 2 should find b and a");
}

#[test]
fn graph_callees_returns_called_functions() {
    let (_dir, repo_path) = init_test_repo(&[
        ("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n"),
    ]);
    let graph = Graph::load(&repo_path).unwrap();

    let main_fn = graph.functions_by_name("main")[0];
    let callees = graph.callees(main_fn);
    let names: Vec<&str> = callees.iter().map(|n| graph.function_name(*n)).collect();
    assert!(names.contains(&"hello"), "main should call hello, got: {names:?}");
}

#[test]
fn graph_references_returns_reference_sites() {
    let (_dir, repo_path) = init_test_repo(&[(
        "src/main.rs",
        "fn main() { register(hello); }\nfn hello() {}\nfn register(_f: fn()) {}\n",
    )]);
    let graph = Graph::load(&repo_path).unwrap();

    let hello = graph.functions_by_name("hello")[0];
    let refs = graph.references(hello);
    assert!(!refs.is_empty(), "hello should be referenced by main");
}

#[test]
fn graph_callers_handles_cycles() {
    let (_dir, repo_path) = init_test_repo(&[(
        "src/main.rs",
        "fn a() { b(); }\nfn b() { a(); }\n",
    )]);
    let graph = Graph::load(&repo_path).unwrap();

    let a = graph.functions_by_name("a")[0];
    // Large depth should not loop forever; cycle detection stops traversal
    let callers = graph.callers(a, 10);
    assert_eq!(callers.len(), 1, "only b should appear (cycle broken), got: {callers:?}");
    assert_eq!(callers[0].function, "b");
}
