use std::process::Command;

use sha2::{Digest, Sha256};

fn create_query_test_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    std::fs::write(
        src_dir.join("main.rs"),
        r#"fn main() {
    router("GET");
    register_handler(validate_body);
}

fn router(method: &str) {
    if method == "POST" {
        validate_body();
    }
    handle(method);
}

fn validate_body() {
    println!("validating");
}

fn register_handler(_handler: fn()) {}

fn handle(method: &str) {}
"#,
    )
    .unwrap();
    std::fs::write(
        tests_dir.join("integration.rs"),
        r#"fn test_validate_body() {
    validate_body();
}
"#,
    )
    .unwrap();

    let run = |args: &[&str]| {
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

    run(&["init"]);
    run(&["config", "user.email", "test@test.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);

    dir
}

fn run_tracegrep(repo: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap()
}

fn run_tracegrep_with_env(
    repo: &std::path::Path,
    envs: &[(&str, &str)],
    args: &[&str],
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tracegrep"));
    command.args(args).current_dir(repo);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().unwrap()
}

fn run_tracegrep_with_cache(
    repo: &std::path::Path,
    cache_dir: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args(args)
        .current_dir(repo)
        .env("TRACEGREP_CACHE_DIR", cache_dir)
        .output()
        .unwrap()
}

fn run_tracegrep_with_cache_and_env(
    repo: &std::path::Path,
    cache_dir: &std::path::Path,
    envs: &[(&str, &str)],
    args: &[&str],
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tracegrep"));
    command
        .args(args)
        .current_dir(repo)
        .env("TRACEGREP_CACHE_DIR", cache_dir);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().unwrap()
}

fn query_cache_root(cache_dir: &std::path::Path, repo: &std::path::Path) -> std::path::PathBuf {
    let repo = repo.canonicalize().unwrap();
    let slug = repo
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("repo");
    let digest = Sha256::digest(repo.to_string_lossy().as_bytes());
    let mut hash = String::new();
    for byte in &digest[..8] {
        use std::fmt::Write;
        let _ = write!(&mut hash, "{byte:02x}");
    }
    cache_dir.join(format!("{slug}-{hash}"))
}

fn query_cache_payload_path(
    cache_dir: &std::path::Path,
    repo: &std::path::Path,
    include_tests: bool,
) -> std::path::PathBuf {
    let suffix = if include_tests { "with-tests" } else { "prod" };
    query_cache_root(cache_dir, repo).join(format!("query-cache.v5.{suffix}.bin"))
}

fn query_cache_meta_path(
    cache_dir: &std::path::Path,
    repo: &std::path::Path,
    include_tests: bool,
) -> std::path::PathBuf {
    let suffix = if include_tests { "with-tests" } else { "prod" };
    query_cache_root(cache_dir, repo).join(format!("query-cache.v5.{suffix}.meta.json"))
}

fn create_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, contents) in files {
        let full_path = dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, contents).unwrap();
    }

    let run = |args: &[&str]| {
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

    run(&["init"]);
    run(&["config", "user.email", "test@test.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);

    dir
}

#[test]
fn test_query_finds_matches_with_call_context() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["validate_body"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("validate_body"), "stdout:\n{stdout}");
    assert!(stdout.contains("router"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("validate_body();\n  Called via:"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn test_query_json_output() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--json", "validate_body"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );

    let mut found_match = false;
    let mut found_enriched_match = false;
    for line in stdout.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("Should be valid JSON: {e}\nLine: {line}"));
        assert!(parsed.get("file").is_some());
        assert!(parsed.get("line").is_some());
        if parsed.get("callers").is_some() {
            found_enriched_match = true;
            assert!(parsed.get("language").is_some());
        }
        found_match = true;
    }
    assert!(found_match);
    assert!(found_enriched_match);
}

#[test]
fn test_query_shows_branch_conditions() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--json", "validate_body"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success());
    for line in stdout.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(callers) = parsed.get("callers").and_then(|callers| callers.as_array()) {
            for caller in callers {
                if caller["function"].as_str() == Some("router") {
                    let conditions = caller["conditions"]
                        .as_array()
                        .expect("conditions should be an array");
                    assert!(!conditions.is_empty(), "caller: {caller}");
                    return;
                }
            }
        }
    }
    panic!("Should have found router caller:\n{stdout}");
}

#[test]
fn test_query_auto_builds_graph() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("main.rs"),
        "fn main() { hello(); }\nfn hello() {}\n",
    )
    .unwrap();

    let run = |args: &[&str]| {
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

    run(&["init"]);
    run(&["config", "user.email", "test@test.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["add", "."]);
    run(&["commit", "-m", "init"]);

    let output = run_tracegrep(dir.path(), &["hello"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(String::from_utf8(output.stdout).unwrap().contains("hello"));
}

#[test]
fn test_query_passes_rg_flags() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["-t", "rust", "validate_body"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("validate_body"));
}

#[test]
fn test_query_accepts_rg_style_positional_path() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["validate_body", "."]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("validate_body"), "stdout:\n{stdout}");
    assert!(stdout.contains("Called via:"), "stdout:\n{stdout}");
}

#[test]
fn test_query_shows_rg_context_lines() {
    let dir = create_repo(&[(
        "src/main.rs",
        r#"fn wrapper() { a(); }

fn a() {
    start();
    middle();
    end();
}

fn start() {}
fn middle() {}
fn end() {}
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["-C", "1", "middle"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("start();"), "stdout:\n{stdout}");
    assert!(stdout.contains("end();"), "stdout:\n{stdout}");
    assert!(stdout.contains("middle();"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("\n4-") || stdout.contains("\n 4-"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("\n5:") || stdout.contains("\n 5:"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("\n6-") || stdout.contains("\n 6-"),
        "stdout:\n{stdout}"
    );
    let end_idx = stdout.find("end();").unwrap();
    let called_via_idx = stdout.find("Called via:").unwrap();
    assert!(end_idx < called_via_idx, "stdout:\n{stdout}");
}

#[test]
fn test_query_shows_before_context_for_subsequent_matches() {
    let dir = create_repo(&[(
        "src/main.rs",
        r#"fn first_match() {}
fn shared_context() {}
fn second_match() {}
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["-C", "1", "first_match|second_match"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(
        stdout.matches("shared_context()").count() >= 2,
        "stdout:\n{stdout}"
    );
}

#[test]
fn test_query_scopes_search_with_positional_path() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["validate_body", "src"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("src/main.rs"), "stdout:\n{stdout}");
    assert!(
        !stdout.contains("tests/integration.rs"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn test_query_accepts_multiple_positional_paths() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["validate_body", "src", "tests"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("src/main.rs"), "stdout:\n{stdout}");
    assert!(stdout.contains("tests/integration.rs"), "stdout:\n{stdout}");
    assert!(stdout.contains("Called via:"), "stdout:\n{stdout}");
}

#[test]
fn test_query_uses_rg_style_colors_when_forced() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--color=always", "validate_body"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("\u{1b}[0m\u{1b}[35m"), "stdout:\n{stdout}");
    assert!(stdout.contains("\u{1b}[0m\u{1b}[32m"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("\u{1b}[0m\u{1b}[1m\u{1b}[31mvalidate_body\u{1b}[0m"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("\u{1b}[0m\u{1b}[2mCalled via:\u{1b}[0m"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn test_query_respects_color_never() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--color=never", "validate_body"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(!stdout.contains('\u{1b}'), "stdout:\n{stdout}");
}

#[test]
fn test_query_hides_test_callers_by_default() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--include-tests", "validate_body"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("1 test caller hidden"), "stdout:\n{stdout}");
    assert!(!stdout.contains("Called via tests:"), "stdout:\n{stdout}");
}

#[test]
fn test_query_can_show_test_callers_explicitly() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(
        dir.path(),
        &["--include-tests", "--include-test-callers", "validate_body"],
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("Called via tests:"), "stdout:\n{stdout}");
    assert!(stdout.contains("test_validate_body"), "stdout:\n{stdout}");
}

#[test]
fn test_query_shows_references_separately_from_callers() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["validate_body"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success());
    assert!(stdout.contains("Referenced by:"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("passed to register_handler"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn test_query_compact_inlines_context_sections() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--compact", "validating"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(
        stdout.contains("src/main.rs:validate_body:13")
            && stdout.contains("[Called via:")
            && stdout.contains("[Referenced by:"),
        "stdout:\n{stdout}"
    );
    assert!(!stdout.contains("\n  Called via:"), "stdout:\n{stdout}");
    assert!(!stdout.contains("\n  Referenced by:"), "stdout:\n{stdout}");
}

#[test]
fn test_query_rebuilds_stale_cache_after_head_changes() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("main.rs"), "fn hello() {}\n").unwrap();

    let run_git = |args: &[&str]| {
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

    run_git(&["init"]);
    run_git(&["config", "user.email", "test@test.com"]);
    run_git(&["config", "user.name", "Test"]);
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "initial"]);

    let first = run_tracegrep(dir.path(), &["hello"]);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8(first.stderr).unwrap()
    );

    std::fs::write(
        src_dir.join("main.rs"),
        "fn main() { hello(); }\nfn hello() {}\n",
    )
    .unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "add caller"]);

    let second = run_tracegrep(dir.path(), &["hello"]);
    let stdout = String::from_utf8(second.stdout).unwrap();
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8(second.stderr).unwrap()
    );
    assert!(
        stdout.contains("Called via:") && stdout.contains("main"),
        "expected rebuilt cache to include new caller, got:\n{stdout}"
    );
}

#[test]
fn test_query_supports_python_callers_and_references() {
    let dir = create_repo(&[(
        "app.py",
        r#"def main():
    router("POST")
    register_handler(validate_body)

def router(method):
    if method == "POST":
        validate_body()

def validate_body():
    print("validating")

def register_handler(handler):
    return handler
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["validate_body"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("app.py"), "stdout:\n{stdout}");
    assert!(stdout.contains("Called via:"), "stdout:\n{stdout}");
    assert!(stdout.contains("router"), "stdout:\n{stdout}");
    assert!(stdout.contains("Referenced by:"), "stdout:\n{stdout}");
    assert!(stdout.contains("register_handler"), "stdout:\n{stdout}");
}

#[test]
fn test_query_supports_typescript_arrow_functions() {
    let dir = create_repo(&[(
        "ui.ts",
        r#"export const render = () => {
  helper();
};

function helper() {
  console.log("ok");
}
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["helper"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("ui.ts"), "stdout:\n{stdout}");
    assert!(stdout.contains("Called via:"), "stdout:\n{stdout}");
    assert!(stdout.contains("render"), "stdout:\n{stdout}");
}

#[test]
fn test_query_supports_jsx_components_and_inline_handlers() {
    let dir = create_repo(&[(
        "Button.jsx",
        r#"function submitForm() {
  console.log("submit");
}

function Button() {
  return <button onClick={() => submitForm()}>{label()}</button>;
}

function label() {
  return "Save";
}
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["submitForm"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("Button.jsx"), "stdout:\n{stdout}");
    assert!(stdout.contains("Called via:"), "stdout:\n{stdout}");
    assert!(stdout.contains("Button"), "stdout:\n{stdout}");
}

#[test]
fn test_query_supports_svelte_template_calls() {
    let dir = create_repo(&[(
        "Button.svelte",
        r#"<script>
  function submitForm() {
    console.log("submit");
  }
</script>

<button on:click={() => submitForm()}>
  Save
</button>
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["submitForm"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("Button.svelte"), "stdout:\n{stdout}");
    assert!(stdout.contains("Called via:"), "stdout:\n{stdout}");
    assert!(stdout.contains("template"), "stdout:\n{stdout}");
}

#[test]
fn test_query_limits_context_by_default_and_prefers_hotter_entries() {
    let dir = create_repo(&[(
        "src/main.rs",
        r#"fn target() {}

fn caller_1() { target(); }
fn caller_2() { target(); }
fn caller_3() { target(); }
fn caller_4() { target(); }
fn caller_5() { target(); }
fn caller_6() { target(); }

fn feeder_1() { caller_6(); }
fn feeder_2() { caller_6(); }

fn sink(_f: fn()) {}
fn register_1() { sink(target); }
fn register_2() { sink(target); }
fn register_3() { sink(target); }
fn register_4() { sink(target); }
fn register_5() { sink(target); }
fn register_6() { sink(target); }

fn ref_feeder_1() { register_6(); }
fn ref_feeder_2() { register_6(); }
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["--json", "^fn target\\("]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );

    let parsed: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    let callers = parsed["callers"].as_array().unwrap();
    let references = parsed["references"].as_array().unwrap();

    assert_eq!(callers.len(), 5, "stdout:\n{stdout}");
    assert_eq!(references.len(), 5, "stdout:\n{stdout}");
    assert_eq!(
        parsed["hidden_callers"].as_u64(),
        Some(1),
        "stdout:\n{stdout}"
    );
    assert_eq!(
        parsed["hidden_references"].as_u64(),
        Some(1),
        "stdout:\n{stdout}"
    );
    assert_eq!(callers[0]["function"].as_str(), Some("caller_6"));
    assert_eq!(references[0]["function"].as_str(), Some("register_6"));
}

#[test]
fn test_query_can_show_more_context_with_max_context() {
    let dir = create_repo(&[(
        "src/main.rs",
        r#"fn target() {}

fn caller_1() { target(); }
fn caller_2() { target(); }
fn caller_3() { target(); }
fn caller_4() { target(); }
fn caller_5() { target(); }
fn caller_6() { target(); }

fn sink(_f: fn()) {}
fn register_1() { sink(target); }
fn register_2() { sink(target); }
fn register_3() { sink(target); }
fn register_4() { sink(target); }
fn register_5() { sink(target); }
fn register_6() { sink(target); }
"#,
    )]);

    let output = run_tracegrep(
        dir.path(),
        &["--json", "--max-context", "10", "^fn target\\("],
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );

    let parsed: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(
        parsed["callers"].as_array().unwrap().len(),
        6,
        "stdout:\n{stdout}"
    );
    assert_eq!(
        parsed["references"].as_array().unwrap().len(),
        6,
        "stdout:\n{stdout}"
    );
    assert!(parsed.get("hidden_callers").is_none(), "stdout:\n{stdout}");
    assert!(
        parsed.get("hidden_references").is_none(),
        "stdout:\n{stdout}"
    );
}

#[test]
fn test_query_does_not_cross_resolve_languages() {
    let dir = create_repo(&[
        (
            "app.py",
            r#"def shared():
    return "py"
"#,
        ),
        (
            "app.ts",
            r#"function shared() {
  return "ts";
}

function render() {
  shared();
}
"#,
        ),
    ]);

    let output = run_tracegrep(dir.path(), &["--json", "def shared|function shared"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );

    let mut saw_python = false;
    let mut saw_typescript = false;
    for line in stdout.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        match parsed["file"].as_str() {
            Some("app.py") => {
                saw_python = true;
                let callers = parsed["callers"].as_array().cloned().unwrap_or_default();
                assert!(
                    callers.is_empty(),
                    "python callers should be empty: {parsed}"
                );
            }
            Some("app.ts") => {
                saw_typescript = true;
                let callers = parsed["callers"].as_array().cloned().unwrap_or_default();
                assert!(callers
                    .iter()
                    .any(|caller| caller["function"].as_str() == Some("render")));
            }
            _ => {}
        }
    }

    assert!(saw_python, "stdout:\n{stdout}");
    assert!(saw_typescript, "stdout:\n{stdout}");
}

#[test]
fn test_build_index_prewarms_cache() {
    let dir = create_repo(&[("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n")]);
    let cache_dir = tempfile::tempdir().unwrap();

    let first = run_tracegrep_with_cache(dir.path(), cache_dir.path(), &["--build-index"]);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8(first.stderr).unwrap()
    );
    let first_stdout = String::from_utf8(first.stdout).unwrap();
    let first_stderr = String::from_utf8(first.stderr).unwrap();
    assert!(
        first_stdout.contains("Built index"),
        "stdout:\n{first_stdout}"
    );
    assert!(
        first_stderr.contains("Building Rust graph"),
        "stderr:\n{first_stderr}"
    );

    let second = run_tracegrep_with_cache(dir.path(), cache_dir.path(), &["--build-index"]);
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8(second.stderr).unwrap()
    );
    let second_stdout = String::from_utf8(second.stdout).unwrap();
    let second_stderr = String::from_utf8(second.stderr).unwrap();
    assert!(
        second_stdout.contains("Index already up to date"),
        "stdout:\n{second_stdout}"
    );
    assert!(
        !second_stderr.contains("Building Rust graph"),
        "stderr:\n{second_stderr}"
    );
}

#[test]
fn test_query_rebuilds_missing_query_cache_from_language_caches() {
    let dir = create_repo(&[("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n")]);
    let cache_dir = tempfile::tempdir().unwrap();

    let build = run_tracegrep_with_cache(dir.path(), cache_dir.path(), &["--build-index"]);
    assert!(build.status.success());

    std::fs::remove_file(query_cache_payload_path(
        cache_dir.path(),
        dir.path(),
        false,
    ))
    .unwrap();
    std::fs::remove_file(query_cache_meta_path(cache_dir.path(), dir.path(), false)).unwrap();

    let output = run_tracegrep_with_cache(dir.path(), cache_dir.path(), &["hello"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("hello"), "stdout:\n{stdout}");
    assert!(query_cache_payload_path(cache_dir.path(), dir.path(), false).exists());
    assert!(query_cache_meta_path(cache_dir.path(), dir.path(), false).exists());
}

#[test]
fn test_query_rebuilds_corrupt_query_cache_from_language_caches() {
    let dir = create_repo(&[("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n")]);
    let cache_dir = tempfile::tempdir().unwrap();

    let build = run_tracegrep_with_cache(dir.path(), cache_dir.path(), &["--build-index"]);
    assert!(build.status.success());

    std::fs::write(
        query_cache_payload_path(cache_dir.path(), dir.path(), false),
        "not bincode",
    )
    .unwrap();

    let output = run_tracegrep_with_cache(dir.path(), cache_dir.path(), &["hello"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(stdout.contains("hello"), "stdout:\n{stdout}");
}

#[test]
fn test_query_timings_env_var_prints_stage_summary() {
    let dir = create_repo(&[("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n")]);
    let cache_dir = tempfile::tempdir().unwrap();

    let output = run_tracegrep_with_cache_and_env(
        dir.path(),
        cache_dir.path(),
        &[("TRACEGREP_TIMINGS", "1")],
        &["hello"],
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "stderr: {}", stderr);
    assert!(
        stderr.contains("tracegrep timings (query):"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("head_hash="), "stderr:\n{stderr}");
    assert!(stderr.contains("query_cache_read="), "stderr:\n{stderr}");
    assert!(stderr.contains("rg_run="), "stderr:\n{stderr}");
    assert!(stderr.contains("total="), "stderr:\n{stderr}");
}

#[test]
fn test_build_index_timings_env_var_prints_stage_summary() {
    let dir = create_repo(&[("src/main.rs", "fn main() { hello(); }\nfn hello() {}\n")]);
    let cache_dir = tempfile::tempdir().unwrap();

    let output = run_tracegrep_with_cache_and_env(
        dir.path(),
        cache_dir.path(),
        &[("TRACEGREP_TIMINGS", "1")],
        &["--build-index"],
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "stderr: {}", stderr);
    assert!(
        stderr.contains("tracegrep timings (build-index):"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("derived_index_build="), "stderr:\n{stderr}");
    assert!(stderr.contains("total="), "stderr:\n{stderr}");
}

#[test]
fn test_generate_zsh_completion_mentions_path_completion() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--generate", "complete-zsh"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(
        stdout.contains("#compdef tg tracegrep"),
        "stdout:\n{stdout}"
    );
    assert!(stdout.contains("_files"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("--install-completions"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn test_install_zsh_completions_writes_files_and_updates_rc() {
    let dir = create_query_test_repo();
    let home = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_string_lossy().into_owned();
    let data_home_str = data_home.path().to_string_lossy().into_owned();

    let output = run_tracegrep_with_env(
        dir.path(),
        &[("HOME", &home_str), ("XDG_DATA_HOME", &data_home_str)],
        &["--install-completions", "zsh"],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );

    let completion_path = data_home
        .path()
        .join("zsh")
        .join("site-functions")
        .join("_tg");
    let rc_path = home.path().join(".zshrc");
    assert!(
        completion_path.exists(),
        "missing {}",
        completion_path.display()
    );
    let completion = std::fs::read_to_string(&completion_path).unwrap();
    assert!(completion.contains("_files"), "completion:\n{completion}");
    let rc = std::fs::read_to_string(&rc_path).unwrap();
    assert!(rc.contains("tracegrep completions"), "rc:\n{rc}");
    let completions_dir = data_home.path().join("zsh").join("site-functions");
    assert!(
        rc.contains(completions_dir.to_string_lossy().as_ref()),
        "rc:\n{rc}"
    );
}

#[test]
fn test_query_location_header_includes_function_definition_line() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--color=never", "validating"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    // validate_body is defined at line 13 in src/main.rs
    // The location header should be file:function:line
    assert!(
        stdout.contains("src/main.rs:validate_body:13"),
        "location header should include function definition line number.\nstdout:\n{stdout}"
    );
}

#[test]
fn test_query_json_output_includes_function_line() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--json", "validating"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );

    for line in stdout.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        if parsed.get("function").is_some() {
            assert!(
                parsed.get("function_line").is_some(),
                "JSON output should include function_line field.\nline: {line}"
            );
            assert_eq!(
                parsed["function_line"].as_u64(),
                Some(13),
                "function_line should be the definition line number.\nline: {line}"
            );
            return;
        }
    }
    panic!("Should have found enriched match with function field.\nstdout:\n{stdout}");
}

#[test]
fn test_query_compact_location_includes_function_definition_line() {
    let dir = create_query_test_repo();
    let output = run_tracegrep(dir.path(), &["--compact", "--color=never", "validating"]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(
        stdout.contains("src/main.rs:validate_body:13"),
        "compact location header should include function definition line number.\nstdout:\n{stdout}"
    );
}

#[test]
fn test_query_finds_calls_inside_macro_invocations() {
    let dir = create_repo(&[(
        "src/main.rs",
        r#"fn run() {
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
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["--json", "handle_result"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );

    // handle_result is called inside tokio::select! — it should show run() as a caller
    let mut found_caller = false;
    for line in stdout.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(callers) = parsed.get("callers").and_then(|c| c.as_array()) {
            for caller in callers {
                if caller["function"].as_str() == Some("run") {
                    found_caller = true;
                }
            }
        }
    }
    assert!(
        found_caller,
        "expected run() to be listed as caller of handle_result via macro body.\nstdout:\n{stdout}"
    );
}

#[test]
fn test_query_finds_calls_inside_assert_macro() {
    let dir = create_repo(&[(
        "src/main.rs",
        r#"fn check() {
    assert!(validate());
    assert_eq!(compute(), 42);
}

fn validate() -> bool { true }
fn compute() -> i32 { 42 }
"#,
    )]);

    let output = run_tracegrep(dir.path(), &["--json", "fn validate"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );

    let mut found_caller = false;
    for line in stdout.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(callers) = parsed.get("callers").and_then(|c| c.as_array()) {
            for caller in callers {
                if caller["function"].as_str() == Some("check") {
                    found_caller = true;
                }
            }
        }
    }
    assert!(
        found_caller,
        "expected check() to be listed as caller of validate via assert! macro.\nstdout:\n{stdout}"
    );
}
