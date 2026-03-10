use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use tracegrep::analysis::codepaths::Language;
use tracegrep::graph_cache::{
    graph_cache_path, load_or_build_graph, query_cache_metadata_path, query_cache_path,
    state_cache_path, state_metadata_path, LoadGraphMode,
};

fn git(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap()
}

fn init_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    for (path, contents) in files {
        let full_path = dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, contents).unwrap();
    }

    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "test@test.com"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "initial"]);

    let repo_path = dir.path().canonicalize().unwrap();
    (dir, repo_path)
}

fn head(repo: &Path) -> String {
    let output = git(repo, &["rev-parse", "HEAD"]);
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn cache_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn cache_reuses_same_head_when_clean() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[("src/main.rs", "fn hello() {}\n")]);

    let first = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(first.outcome.mode, LoadGraphMode::FullRebuild);

    let second = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(second.outcome.mode, LoadGraphMode::Reused);
    assert!(second.outcome.changed_files.is_empty());
}

#[test]
fn cache_reuses_on_docs_only_head_change_and_updates_state() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[
        ("src/main.rs", "fn hello() {}\n"),
        ("README.md", "initial\n"),
    ]);

    load_or_build_graph(&repo_path, false).unwrap();
    std::fs::write(repo_path.join("README.md"), "updated\n").unwrap();
    git(&repo_path, &["add", "README.md"]);
    git(&repo_path, &["commit", "-m", "docs"]);

    let result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(result.outcome.mode, LoadGraphMode::Reused);
    assert!(result.outcome.changed_files.is_empty());

    let state_path = state_metadata_path(&repo_path, false, Language::Rust).unwrap();
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(state_path).unwrap()).unwrap();
    let expected_head = head(&repo_path);
    assert_eq!(state["head"].as_str(), Some(expected_head.as_str()));
}

#[test]
fn cache_incrementally_updates_modified_tracked_production_file() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[("src/main.rs", "fn hello() {}\n")]);

    load_or_build_graph(&repo_path, false).unwrap();
    std::fs::write(
        repo_path.join("src/main.rs"),
        "fn main() { hello(); }\nfn hello() {}\n",
    )
    .unwrap();

    let result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(result.outcome.mode, LoadGraphMode::Incremental);
    assert_eq!(result.outcome.changed_files, vec!["src/main.rs"]);
}

#[test]
fn cache_incrementally_updates_untracked_production_file() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[("src/main.rs", "fn hello() {}\n")]);

    load_or_build_graph(&repo_path, false).unwrap();
    std::fs::write(repo_path.join("src/extra.rs"), "fn helper() {}\n").unwrap();

    let result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(result.outcome.mode, LoadGraphMode::Incremental);
    assert_eq!(result.outcome.changed_files, vec!["src/extra.rs"]);
}

#[test]
fn cache_incrementally_updates_deleted_production_file() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[
        (
            "src/main.rs",
            "mod helper;\nfn main() { helper::hello(); }\n",
        ),
        ("src/helper.rs", "pub fn hello() {}\n"),
    ]);

    load_or_build_graph(&repo_path, false).unwrap();
    std::fs::remove_file(repo_path.join("src/helper.rs")).unwrap();
    std::fs::write(repo_path.join("src/main.rs"), "fn main() {}\n").unwrap();

    let result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(result.outcome.mode, LoadGraphMode::Incremental);
    assert!(result
        .outcome
        .changed_files
        .contains(&"src/helper.rs".to_string()));
}

#[test]
fn production_cache_ignores_test_only_changes() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[
        ("src/main.rs", "fn hello() {}\n"),
        (
            "tests/integration.rs",
            "#[test]\nfn it_works() { hello(); }\n",
        ),
    ]);

    load_or_build_graph(&repo_path, false).unwrap();
    std::fs::write(
        repo_path.join("tests/integration.rs"),
        "#[test]\nfn it_works() { assert!(true); }\n",
    )
    .unwrap();

    let result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(result.outcome.mode, LoadGraphMode::Reused);
    assert!(result.outcome.changed_files.is_empty());
}

#[test]
fn include_tests_cache_tracks_test_file_changes() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[
        ("src/main.rs", "fn hello() {}\n"),
        (
            "tests/integration.rs",
            "#[test]\nfn it_works() { hello(); }\n",
        ),
    ]);

    load_or_build_graph(&repo_path, true).unwrap();
    std::fs::write(
        repo_path.join("tests/integration.rs"),
        "#[test]\nfn it_works() { assert!(true); }\n",
    )
    .unwrap();

    let result = load_or_build_graph(&repo_path, true).unwrap();
    assert_eq!(result.outcome.mode, LoadGraphMode::Incremental);
    assert_eq!(result.outcome.changed_files, vec!["tests/integration.rs"]);
}

#[test]
fn corrupt_state_or_graph_falls_back_to_full_rebuild() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[("src/main.rs", "fn hello() {}\n")]);

    load_or_build_graph(&repo_path, false).unwrap();
    std::fs::remove_file(query_cache_path(&repo_path, false).unwrap()).unwrap();
    std::fs::remove_file(query_cache_metadata_path(&repo_path, false).unwrap()).unwrap();
    std::fs::write(
        state_metadata_path(&repo_path, false, Language::Rust).unwrap(),
        "{not json",
    )
    .unwrap();
    let state_result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(state_result.outcome.mode, LoadGraphMode::FullRebuild);

    load_or_build_graph(&repo_path, false).unwrap();
    std::fs::remove_file(query_cache_path(&repo_path, false).unwrap()).unwrap();
    std::fs::remove_file(query_cache_metadata_path(&repo_path, false).unwrap()).unwrap();
    std::fs::write(
        graph_cache_path(&repo_path, false, Language::Rust).unwrap(),
        "{not json",
    )
    .unwrap();
    let graph_result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(graph_result.outcome.mode, LoadGraphMode::FullRebuild);
}

#[test]
fn cache_writes_separate_files_per_language() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[
        ("src/main.rs", "fn hello() {}\n"),
        ("app.py", "def greet():\n    pass\n"),
        (
            "Component.svelte",
            "<script>function render() {}</script>\n",
        ),
        ("ui.ts", "export function render() {}\n"),
    ]);

    let result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(result.outcome.mode, LoadGraphMode::FullRebuild);

    assert!(graph_cache_path(&repo_path, false, Language::Rust)
        .unwrap()
        .exists());
    assert!(graph_cache_path(&repo_path, false, Language::Python)
        .unwrap()
        .exists());
    assert!(graph_cache_path(&repo_path, false, Language::Svelte)
        .unwrap()
        .exists());
    assert!(graph_cache_path(&repo_path, false, Language::TypeScript)
        .unwrap()
        .exists());
}

#[test]
fn cache_only_rebuilds_changed_language_files() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[
        ("src/main.rs", "fn hello() {}\n"),
        ("app.py", "def greet():\n    pass\n"),
    ]);

    load_or_build_graph(&repo_path, false).unwrap();
    std::fs::write(
        repo_path.join("app.py"),
        "def main():\n    greet()\n\ndef greet():\n    pass\n",
    )
    .unwrap();

    let result = load_or_build_graph(&repo_path, false).unwrap();
    assert_eq!(result.outcome.mode, LoadGraphMode::Incremental);
    assert_eq!(result.outcome.changed_files, vec!["app.py"]);
}

#[test]
fn cache_dir_can_be_overridden_with_env_var() {
    let _guard = cache_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, repo_path) = init_repo(&[("src/main.rs", "fn hello() {}\n")]);
    let override_dir = tempfile::tempdir().unwrap();

    std::env::set_var("TRACEGREP_CACHE_DIR", override_dir.path());
    let graph_path = graph_cache_path(&repo_path, false, Language::Rust).unwrap();
    let state_path = state_cache_path(&repo_path, false, Language::Rust).unwrap();
    let result = load_or_build_graph(&repo_path, false).unwrap();
    std::env::remove_var("TRACEGREP_CACHE_DIR");

    assert_eq!(result.outcome.mode, LoadGraphMode::FullRebuild);
    assert!(graph_path.starts_with(override_dir.path()));
    assert!(state_path.starts_with(override_dir.path()));
    assert!(graph_path.exists());
    assert!(state_path.exists());
}
