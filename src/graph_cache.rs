use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::analysis;
use crate::analysis::codepaths::{self, merge_graphs, CallGraph, FileArtifact, Language};

const CACHE_DIR_ENV: &str = "TRACEGREP_CACHE_DIR";
const CACHE_DIR: &str = ".cache/tracegrep";
const SCHEMA_VERSION: u32 = 4;

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
    language: Language,
    head: String,
    files: BTreeMap<String, FileArtifact>,
}

struct LanguageLoadResult {
    graph: CallGraph,
    outcome: LoadGraphOutcome,
}

fn empty_graph() -> CallGraph {
    CallGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
        references: Vec::new(),
    }
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
    let cache_root = std::env::var_os(CACHE_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").expect("HOME is not set");
            PathBuf::from(home).join(CACHE_DIR)
        });
    let repo_key = repo_path.to_string_lossy();
    let slug = repo_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("repo");
    let hash = &hex_hash(repo_key.as_bytes())[..16];
    Ok(cache_root.join(format!("{slug}-{hash}")))
}

pub fn graph_cache_path(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> anyhow::Result<PathBuf> {
    let cache_dir = repo_cache_dir(repo_path)?;
    let suffix = if include_tests { "with-tests" } else { "prod" };
    Ok(cache_dir.join(format!(
        "codepaths.v{SCHEMA_VERSION}.{}.{}.graph",
        language.cache_key(),
        suffix
    )))
}

pub fn state_cache_path(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> anyhow::Result<PathBuf> {
    let cache_dir = repo_cache_dir(repo_path)?;
    let suffix = if include_tests { "with-tests" } else { "prod" };
    Ok(cache_dir.join(format!(
        "codepaths.v{SCHEMA_VERSION}.{}.{}.state.json",
        language.cache_key(),
        suffix
    )))
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

fn read_graph(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> anyhow::Result<CallGraph> {
    let path = graph_cache_path(repo_path, include_tests, language)?;
    let data = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

fn write_graph(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
    graph: &CallGraph,
) -> anyhow::Result<()> {
    let path = graph_cache_path(repo_path, include_tests, language)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string(graph)?)?;
    Ok(())
}

fn read_state(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> anyhow::Result<CacheState> {
    let path = state_cache_path(repo_path, include_tests, language)?;
    let data = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

fn write_state(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
    state: &CacheState,
) -> anyhow::Result<()> {
    let path = state_cache_path(repo_path, include_tests, language)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string(state)?)?;
    Ok(())
}

fn is_relevant_path(path: &str, include_tests: bool, language: Language) -> bool {
    let extension_language = codepaths::language_for_path(Path::new(path));
    extension_language == Some(language) && (include_tests || !analysis::is_test_file(path))
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

fn parse_name_status_z(output: &[u8], include_tests: bool, language: Language) -> Vec<String> {
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
            if is_relevant_path(&old_path, include_tests, language) {
                changed.push(old_path);
            }
            if is_relevant_path(&new_path, include_tests, language) {
                changed.push(new_path);
            }
            continue;
        }

        let Some(path) = parts.next() else {
            break;
        };
        let path = String::from_utf8_lossy(path).into_owned();
        if is_relevant_path(&path, include_tests, language) {
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
    language: Language,
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
    Ok(parse_name_status_z(&output, include_tests, language))
}

fn dirty_changed_paths(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> anyhow::Result<Vec<String>> {
    let mut changed = parse_name_status_z(
        &run_git_bytes(repo_path, &["diff", "--name-status", "-z", "HEAD", "--"])?,
        include_tests,
        language,
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
            .filter(|path| is_relevant_path(path, include_tests, language)),
    );

    Ok(changed)
}

fn build_file_artifact(
    relative_path: &str,
    path: &Path,
    include_tests: bool,
    language: Language,
    parser: &mut tree_sitter::Parser,
) -> anyhow::Result<FileArtifact> {
    let source = std::fs::read_to_string(path)?;
    let mut artifact =
        codepaths::extract_from_source(&source, relative_path, include_tests, language, parser)
            .ok_or_else(|| anyhow::anyhow!("failed to parse {}", path.display()))?;
    artifact.source_hash = hex_hash(source.as_bytes());
    Ok(artifact)
}

fn build_all_artifacts(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> anyhow::Result<BTreeMap<String, FileArtifact>> {
    let mut parser = codepaths::new_parser(language)?;
    let mut files = BTreeMap::new();

    for (relative_path, path) in
        codepaths::collect_relevant_files_for_language(repo_path, include_tests, language)
    {
        files.insert(
            relative_path.clone(),
            build_file_artifact(&relative_path, &path, include_tests, language, &mut parser)?,
        );
    }

    Ok(files)
}

fn full_rebuild_language(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
    language: Language,
) -> anyhow::Result<LanguageLoadResult> {
    eprintln!("Building {} graph...", language.display_name());
    let files = build_all_artifacts(repo_path, include_tests, language)?;
    let graph = codepaths::build_graph_from_artifacts(&files);
    let changed_files = files.keys().cloned().collect::<Vec<_>>();

    write_graph(repo_path, include_tests, language, &graph)?;
    write_state(
        repo_path,
        include_tests,
        language,
        &CacheState {
            schema_version: SCHEMA_VERSION,
            repo_path: repo_path.display().to_string(),
            include_tests,
            language,
            head: current_head.to_string(),
            files,
        },
    )?;

    Ok(LanguageLoadResult {
        graph,
        outcome: LoadGraphOutcome {
            mode: LoadGraphMode::FullRebuild,
            changed_files,
        },
    })
}

fn incremental_rebuild_language(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
    language: Language,
    mut state: CacheState,
    changed_files: &[String],
) -> anyhow::Result<LanguageLoadResult> {
    let mut parser = codepaths::new_parser(language)?;
    let current_files =
        codepaths::collect_relevant_files_for_language(repo_path, include_tests, language);

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
            let artifact =
                build_file_artifact(changed, path, include_tests, language, &mut parser)?;
            state.files.insert(changed.clone(), artifact);
        } else {
            state.files.remove(changed);
        }
    }

    let graph = codepaths::build_graph_from_artifacts(&state.files);
    state.head = current_head.to_string();

    write_graph(repo_path, include_tests, language, &graph)?;
    write_state(repo_path, include_tests, language, &state)?;
    eprintln!(
        "Incrementally rebuilding {} graph ({} changed file{})",
        language.display_name(),
        changed_files.len(),
        if changed_files.len() == 1 { "" } else { "s" }
    );

    Ok(LanguageLoadResult {
        graph,
        outcome: LoadGraphOutcome {
            mode: LoadGraphMode::Incremental,
            changed_files: changed_files.to_vec(),
        },
    })
}

fn load_or_build_graph_for_language(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
    language: Language,
) -> anyhow::Result<Option<LanguageLoadResult>> {
    let current_files =
        codepaths::collect_relevant_files_for_language(repo_path, include_tests, language);
    let state = read_state(repo_path, include_tests, language);
    let graph = read_graph(repo_path, include_tests, language);
    let has_cache = state.is_ok() || graph.is_ok();

    if current_files.is_empty() && !has_cache {
        return Ok(None);
    }

    let (mut state, graph) = match (state, graph) {
        (Ok(state), Ok(graph))
            if state.schema_version == SCHEMA_VERSION
                && state.repo_path == repo_path.display().to_string()
                && state.include_tests == include_tests
                && state.language == language =>
        {
            (state, graph)
        }
        _ => {
            return Ok(Some(full_rebuild_language(
                repo_path,
                include_tests,
                current_head,
                language,
            )?))
        }
    };

    let mut changed_files = committed_changed_paths(
        repo_path,
        &state.head,
        current_head,
        include_tests,
        language,
    )?;
    changed_files.extend(dirty_changed_paths(repo_path, include_tests, language)?);
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
            state.head = current_head.to_string();
            write_state(repo_path, include_tests, language, &state)?;
            eprintln!(
                "Reusing cached {} graph (HEAD changed, no relevant source changes)",
                language.display_name()
            );
        } else {
            eprintln!("Reusing cached {} graph", language.display_name());
        }
        return Ok(Some(LanguageLoadResult {
            graph,
            outcome: LoadGraphOutcome {
                mode: LoadGraphMode::Reused,
                changed_files,
            },
        }));
    }

    Ok(Some(incremental_rebuild_language(
        repo_path,
        include_tests,
        current_head,
        language,
        state,
        &changed_files,
    )?))
}

pub fn load_or_build_graph(
    repo_path: &Path,
    include_tests: bool,
) -> anyhow::Result<LoadGraphResult> {
    let current_head = head_hash(repo_path)?;
    let mut graphs = Vec::new();
    let mut changed_files = Vec::new();
    let mut mode = LoadGraphMode::Reused;

    for language in Language::ALL {
        let Some(result) =
            load_or_build_graph_for_language(repo_path, include_tests, &current_head, language)?
        else {
            continue;
        };

        if !result.graph.nodes.is_empty()
            || !result.graph.edges.is_empty()
            || !result.graph.references.is_empty()
        {
            graphs.push(result.graph);
        }
        changed_files.extend(result.outcome.changed_files);
        mode = merge_mode(mode, result.outcome.mode);
    }

    changed_files.sort();
    changed_files.dedup();

    Ok(LoadGraphResult {
        graph: if graphs.is_empty() {
            empty_graph()
        } else {
            merge_graphs(&graphs)
        },
        outcome: LoadGraphOutcome {
            mode,
            changed_files,
        },
    })
}

fn merge_mode(current: LoadGraphMode, next: LoadGraphMode) -> LoadGraphMode {
    match (current, next) {
        (LoadGraphMode::FullRebuild, _) | (_, LoadGraphMode::FullRebuild) => {
            LoadGraphMode::FullRebuild
        }
        (LoadGraphMode::Incremental, _) | (_, LoadGraphMode::Incremental) => {
            LoadGraphMode::Incremental
        }
        _ => LoadGraphMode::Reused,
    }
}
