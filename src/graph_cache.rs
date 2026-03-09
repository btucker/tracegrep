use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::analysis;
use crate::analysis::codepaths::{self, CallGraph, FileArtifact};

const CACHE_DIR: &str = ".cache/tracegrep";
const SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadGraphMode {
    Reused,
    Incremental,
    FullRebuild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadGraphOutcome {
    pub mode: LoadGraphMode,
    pub changed_files: Vec<String>,
}

pub struct LoadGraphResult {
    pub graph: CallGraph,
    pub outcome: LoadGraphOutcome,
}

#[derive(Serialize, Deserialize)]
struct CacheState {
    schema_version: u32,
    repo_path: String,
    include_tests: bool,
    head: String,
    files: BTreeMap<String, FileArtifact>,
}

fn hex_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn repo_cache_dir(repo_path: &Path) -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    let repo_key = repo_path.to_string_lossy();
    let slug = repo_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("repo");
    let hash = &hex_hash(repo_key.as_bytes())[..16];
    Ok(PathBuf::from(home)
        .join(CACHE_DIR)
        .join(format!("{slug}-{hash}")))
}

pub fn graph_cache_path(repo_path: &Path, include_tests: bool) -> anyhow::Result<PathBuf> {
    let cache_dir = repo_cache_dir(repo_path)?;
    if include_tests {
        Ok(cache_dir.join("codepaths.v3.graph.with-tests"))
    } else {
        Ok(cache_dir.join("codepaths.v3.graph"))
    }
}

pub fn state_cache_path(repo_path: &Path, include_tests: bool) -> anyhow::Result<PathBuf> {
    let cache_dir = repo_cache_dir(repo_path)?;
    if include_tests {
        Ok(cache_dir.join("codepaths.v3.state.with-tests.json"))
    } else {
        Ok(cache_dir.join("codepaths.v3.state.json"))
    }
}

fn head_hash(repo_path: &Path) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("Failed to read git HEAD in {}", repo_path.display());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn read_graph(repo_path: &Path, include_tests: bool) -> anyhow::Result<CallGraph> {
    let path = graph_cache_path(repo_path, include_tests)?;
    let data = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

fn write_graph(repo_path: &Path, include_tests: bool, graph: &CallGraph) -> anyhow::Result<()> {
    let path = graph_cache_path(repo_path, include_tests)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string(graph)?)?;
    Ok(())
}

fn read_state(repo_path: &Path, include_tests: bool) -> anyhow::Result<CacheState> {
    let path = state_cache_path(repo_path, include_tests)?;
    let data = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

fn write_state(repo_path: &Path, include_tests: bool, state: &CacheState) -> anyhow::Result<()> {
    let path = state_cache_path(repo_path, include_tests)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string(state)?)?;
    Ok(())
}

fn is_relevant_rust_path(path: &str, include_tests: bool) -> bool {
    path.ends_with(".rs") && (include_tests || !analysis::is_test_file(path))
}

fn run_git_bytes(repo_path: &Path, args: &[&str]) -> anyhow::Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("git {} failed in {}", args.join(" "), repo_path.display());
    }
    Ok(output.stdout)
}

fn parse_name_status_z(output: &[u8], include_tests: bool) -> Vec<String> {
    let mut parts = output
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty());
    let mut changed = Vec::new();

    while let Some(status) = parts.next() {
        let status = String::from_utf8_lossy(status);
        if status.starts_with('R') || status.starts_with('C') {
            let Some(old_path) = parts.next() else {
                break;
            };
            let Some(new_path) = parts.next() else {
                break;
            };
            let old_path = String::from_utf8_lossy(old_path).into_owned();
            let new_path = String::from_utf8_lossy(new_path).into_owned();
            if is_relevant_rust_path(&old_path, include_tests) {
                changed.push(old_path);
            }
            if is_relevant_rust_path(&new_path, include_tests) {
                changed.push(new_path);
            }
            continue;
        }

        let Some(path) = parts.next() else {
            break;
        };
        let path = String::from_utf8_lossy(path).into_owned();
        if is_relevant_rust_path(&path, include_tests) {
            changed.push(path);
        }
    }

    changed
}

fn committed_changed_paths(
    repo_path: &Path,
    previous_head: &str,
    current_head: &str,
    include_tests: bool,
) -> anyhow::Result<Vec<String>> {
    if previous_head == current_head {
        return Ok(Vec::new());
    }

    let output = run_git_bytes(
        repo_path,
        &[
            "diff",
            "--name-status",
            "-z",
            previous_head,
            current_head,
            "--",
        ],
    )?;
    Ok(parse_name_status_z(&output, include_tests))
}

fn dirty_changed_paths(repo_path: &Path, include_tests: bool) -> anyhow::Result<Vec<String>> {
    let mut changed = parse_name_status_z(
        &run_git_bytes(repo_path, &["diff", "--name-status", "-z", "HEAD", "--"])?,
        include_tests,
    );

    let untracked = run_git_bytes(
        repo_path,
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )?;
    changed.extend(
        untracked
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .filter_map(|path| String::from_utf8(path.to_vec()).ok())
            .filter(|path| is_relevant_rust_path(path, include_tests)),
    );

    Ok(changed)
}

fn build_file_artifact(
    parser: &mut tree_sitter::Parser,
    relative_path: &str,
    path: &Path,
    include_tests: bool,
) -> anyhow::Result<FileArtifact> {
    let source = std::fs::read_to_string(path)?;
    let mut artifact =
        codepaths::extract_from_source(&source, relative_path, include_tests, parser)
            .ok_or_else(|| anyhow::anyhow!("failed to parse {}", path.display()))?;
    artifact.source_hash = hex_hash(source.as_bytes());
    Ok(artifact)
}

fn build_all_artifacts(
    repo_path: &Path,
    include_tests: bool,
) -> anyhow::Result<BTreeMap<String, FileArtifact>> {
    let mut parser = codepaths::new_parser()?;
    let mut files = BTreeMap::new();

    for (relative_path, path) in codepaths::collect_relevant_rust_files(repo_path, include_tests) {
        files.insert(
            relative_path.clone(),
            build_file_artifact(&mut parser, &relative_path, &path, include_tests)?,
        );
    }

    Ok(files)
}

fn full_rebuild(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
) -> anyhow::Result<LoadGraphResult> {
    eprintln!("Building call graph...");
    let files = build_all_artifacts(repo_path, include_tests)?;
    let graph = codepaths::build_graph_from_artifacts(&files);
    eprintln!(
        "Call graph: {} nodes, {} edges, {} references",
        graph.nodes.len(),
        graph.edges.len(),
        graph.references.len()
    );

    let changed_files = files.keys().cloned().collect::<Vec<_>>();
    write_graph(repo_path, include_tests, &graph)?;
    write_state(
        repo_path,
        include_tests,
        &CacheState {
            schema_version: SCHEMA_VERSION,
            repo_path: repo_path.display().to_string(),
            include_tests,
            head: current_head.to_string(),
            files,
        },
    )?;
    eprintln!(
        "Call graph cached to {}",
        graph_cache_path(repo_path, include_tests)?.display()
    );

    Ok(LoadGraphResult {
        graph,
        outcome: LoadGraphOutcome {
            mode: LoadGraphMode::FullRebuild,
            changed_files,
        },
    })
}

fn incremental_rebuild(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
    mut state: CacheState,
    changed_files: &[String],
) -> anyhow::Result<LoadGraphResult> {
    let mut parser = codepaths::new_parser()?;
    let current_files = codepaths::collect_relevant_rust_files(repo_path, include_tests);

    for removed in state
        .files
        .keys()
        .filter(|path| !current_files.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>()
    {
        state.files.remove(&removed);
    }

    for changed in changed_files {
        if let Some(path) = current_files.get(changed) {
            let artifact = build_file_artifact(&mut parser, changed, path, include_tests)?;
            state.files.insert(changed.clone(), artifact);
        } else {
            state.files.remove(changed);
        }
    }

    let graph = codepaths::build_graph_from_artifacts(&state.files);
    state.head = current_head.to_string();

    write_graph(repo_path, include_tests, &graph)?;
    write_state(repo_path, include_tests, &state)?;
    eprintln!(
        "Incrementally rebuilding call graph ({} changed file{})",
        changed_files.len(),
        if changed_files.len() == 1 { "" } else { "s" }
    );

    Ok(LoadGraphResult {
        graph,
        outcome: LoadGraphOutcome {
            mode: LoadGraphMode::Incremental,
            changed_files: changed_files.to_vec(),
        },
    })
}

pub fn load_or_build_graph(
    repo_path: &Path,
    include_tests: bool,
) -> anyhow::Result<LoadGraphResult> {
    let current_head = head_hash(repo_path)?;
    let state = read_state(repo_path, include_tests);
    let graph = read_graph(repo_path, include_tests);

    let (mut state, graph) = match (state, graph) {
        (Ok(state), Ok(graph))
            if state.schema_version == SCHEMA_VERSION
                && state.repo_path == repo_path.display().to_string()
                && state.include_tests == include_tests =>
        {
            (state, graph)
        }
        _ => return full_rebuild(repo_path, include_tests, &current_head),
    };

    let current_files = codepaths::collect_relevant_rust_files(repo_path, include_tests);
    let mut changed_files =
        committed_changed_paths(repo_path, &state.head, &current_head, include_tests)?;
    changed_files.extend(dirty_changed_paths(repo_path, include_tests)?);
    changed_files.extend(
        state
            .files
            .keys()
            .filter(|path| !current_files.contains_key(*path))
            .cloned(),
    );
    changed_files.extend(
        current_files
            .keys()
            .filter(|path| !state.files.contains_key(*path))
            .cloned(),
    );
    changed_files.sort();
    changed_files.dedup();

    if changed_files.is_empty() {
        if state.head != current_head {
            state.head = current_head;
            write_state(repo_path, include_tests, &state)?;
            eprintln!("Reusing cached call graph (HEAD changed, no relevant Rust changes)");
        } else {
            eprintln!("Reusing cached call graph");
        }
        return Ok(LoadGraphResult {
            graph,
            outcome: LoadGraphOutcome {
                mode: LoadGraphMode::Reused,
                changed_files,
            },
        });
    }

    incremental_rebuild(
        repo_path,
        include_tests,
        &current_head,
        state,
        &changed_files,
    )
}
