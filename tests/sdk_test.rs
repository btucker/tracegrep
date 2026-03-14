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
        Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap()
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
    let (_dir, repo_path) = init_test_repo(&[
        ("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n"),
    ]);
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
    let with = Graph::builder(&repo_path).include_tests(true).build().unwrap();

    assert!(with.node_count() > without.node_count(),
        "with_tests={} should be > without={}",
        with.node_count(), without.node_count());
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
