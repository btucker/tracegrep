use std::process::Command;

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

#[test]
fn test_query_finds_matches_with_call_context() {
    let dir = create_query_test_repo();
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args(["validate_body", "--repo", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args([
            "--json",
            "validate_body",
            "--repo",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

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
        }
        found_match = true;
    }
    assert!(found_match);
    assert!(found_enriched_match);
}

#[test]
fn test_query_shows_branch_conditions() {
    let dir = create_query_test_repo();
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args([
            "--json",
            "validate_body",
            "--repo",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

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

    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args(["hello", "--repo", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args([
            "validate_body",
            "--repo",
            dir.path().to_str().unwrap(),
            "-t",
            "rust",
        ])
        .output()
        .unwrap();

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
fn test_query_uses_rg_style_colors_when_forced() {
    let dir = create_query_test_repo();
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args([
            "validate_body",
            "--repo",
            dir.path().to_str().unwrap(),
            "--color=always",
        ])
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args([
            "validate_body",
            "--repo",
            dir.path().to_str().unwrap(),
            "--color=never",
        ])
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args([
            "--include-tests",
            "validate_body",
            "--repo",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args([
            "--include-tests",
            "--include-test-callers",
            "validate_body",
            "--repo",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args(["validate_body", "--repo", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args([
            "--compact",
            "validating",
            "--repo",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(
        stdout.contains("validate_body:")
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

    let first = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args(["hello", "--repo", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
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

    let second = Command::new(env!("CARGO_BIN_EXE_tracegrep"))
        .args(["hello", "--repo", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
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
