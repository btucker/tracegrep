use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::analysis;
use crate::analysis::codepaths::{self, merge_graphs, CallGraph, FileArtifact, Language};
use crate::query_data::QueryCachePayload;
use crate::timing::TimingCollector;

const CACHE_DIR_ENV: &str = "TRACEGREP_CACHE_DIR";
const CACHE_DIR: &str = ".cache/tracegrep";
const SCHEMA_VERSION: u32 = 5;

/// A guard that prints a message to stderr after a delay.
/// If dropped before the delay elapses, the message is suppressed.
struct DelayedMessage {
    cancel: Arc<AtomicBool>,
}

impl DelayedMessage {
    fn new(message: String, delay: std::time::Duration) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if !cancel_clone.load(Ordering::Relaxed) {
                eprintln!("{message}");
            }
        });
        Self { cancel }
    }
}

impl Drop for DelayedMessage {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

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

pub struct LoadQueryResult {
    pub payload: QueryCachePayload,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StateMetadata {
    schema_version: u32,
    repo_path: String,
    include_tests: bool,
    language: Language,
    head: String,
    content_fingerprint: String,
    file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueryCacheMetadata {
    schema_version: u32,
    repo_path: String,
    include_tests: bool,
    head: String,
    state_fingerprint: String,
}

struct LanguageLoadResult {
    graph: CallGraph,
    state_meta: StateMetadata,
    outcome: LoadGraphOutcome,
}

struct LanguageCachePlan {
    language: Language,
    current_files: BTreeMap<String, PathBuf>,
    state_meta: Option<StateMetadata>,
    action: LanguageCacheAction,
}

enum LanguageCacheAction {
    Skip,
    Reuse,
    RefreshHeadOnly,
    Incremental { changed_files: Vec<String> },
    FullRebuild,
}

struct RepoStatusSnapshot {
    current_head: String,
    shared_previous_head: Option<String>,
    committed_paths: Vec<String>,
    dirty_paths: Vec<String>,
    untracked_paths: Vec<String>,
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

pub fn state_metadata_path(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> anyhow::Result<PathBuf> {
    let cache_dir = repo_cache_dir(repo_path)?;
    let suffix = if include_tests { "with-tests" } else { "prod" };
    Ok(cache_dir.join(format!(
        "codepaths.v{SCHEMA_VERSION}.{}.{}.state-meta.json",
        language.cache_key(),
        suffix
    )))
}

pub fn query_cache_path(repo_path: &Path, include_tests: bool) -> anyhow::Result<PathBuf> {
    let cache_dir = repo_cache_dir(repo_path)?;
    let suffix = if include_tests { "with-tests" } else { "prod" };
    Ok(cache_dir.join(format!("query-cache.v{SCHEMA_VERSION}.{suffix}.bin")))
}

pub fn query_cache_metadata_path(repo_path: &Path, include_tests: bool) -> anyhow::Result<PathBuf> {
    let cache_dir = repo_cache_dir(repo_path)?;
    let suffix = if include_tests { "with-tests" } else { "prod" };
    Ok(cache_dir.join(format!("query-cache.v{SCHEMA_VERSION}.{suffix}.meta.json")))
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
    timings: &mut TimingCollector,
) -> anyhow::Result<CallGraph> {
    let path = graph_cache_path(repo_path, include_tests, language)?;
    timings.measure("graph_read", || -> anyhow::Result<CallGraph> {
        let file = File::open(path)?;
        Ok(serde_json::from_reader(BufReader::new(file))?)
    })
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
    let file = File::create(path)?;
    serde_json::to_writer(BufWriter::new(file), graph)?;
    Ok(())
}

fn read_state(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
    timings: &mut TimingCollector,
) -> anyhow::Result<CacheState> {
    let path = state_cache_path(repo_path, include_tests, language)?;
    timings.measure("state_read", || -> anyhow::Result<CacheState> {
        let file = File::open(path)?;
        Ok(serde_json::from_reader(BufReader::new(file))?)
    })
}

fn write_state(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
    state: &CacheState,
) -> anyhow::Result<StateMetadata> {
    let path = state_cache_path(repo_path, include_tests, language)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    serde_json::to_writer(BufWriter::new(file), state)?;

    let meta = state_metadata_from_state(state);
    write_state_metadata(repo_path, include_tests, language, &meta)?;
    Ok(meta)
}

fn read_state_metadata(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
    timings: &mut TimingCollector,
) -> anyhow::Result<StateMetadata> {
    let path = state_metadata_path(repo_path, include_tests, language)?;
    timings.measure("state_read", || -> anyhow::Result<StateMetadata> {
        let file = File::open(path)?;
        Ok(serde_json::from_reader(BufReader::new(file))?)
    })
}

fn write_state_metadata(
    repo_path: &Path,
    include_tests: bool,
    language: Language,
    meta: &StateMetadata,
) -> anyhow::Result<()> {
    let path = state_metadata_path(repo_path, include_tests, language)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    serde_json::to_writer(BufWriter::new(file), meta)?;
    Ok(())
}

fn read_query_cache_metadata(
    repo_path: &Path,
    include_tests: bool,
    timings: &mut TimingCollector,
) -> anyhow::Result<QueryCacheMetadata> {
    let path = query_cache_metadata_path(repo_path, include_tests)?;
    timings.measure(
        "query_cache_read",
        || -> anyhow::Result<QueryCacheMetadata> {
            let file = File::open(path)?;
            Ok(serde_json::from_reader(BufReader::new(file))?)
        },
    )
}

fn write_query_cache_metadata(
    repo_path: &Path,
    include_tests: bool,
    meta: &QueryCacheMetadata,
) -> anyhow::Result<()> {
    let path = query_cache_metadata_path(repo_path, include_tests)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    serde_json::to_writer(BufWriter::new(file), meta)?;
    Ok(())
}

fn read_query_cache_payload(
    repo_path: &Path,
    include_tests: bool,
    timings: &mut TimingCollector,
) -> anyhow::Result<QueryCachePayload> {
    let path = query_cache_path(repo_path, include_tests)?;
    timings.measure(
        "query_cache_read",
        || -> anyhow::Result<QueryCachePayload> {
            let file = File::open(path)?;
            Ok(bincode::deserialize_from(BufReader::new(file))?)
        },
    )
}

fn write_query_cache_payload(
    repo_path: &Path,
    include_tests: bool,
    payload: &QueryCachePayload,
) -> anyhow::Result<()> {
    let path = query_cache_path(repo_path, include_tests)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    bincode::serialize_into(BufWriter::new(file), payload)?;
    Ok(())
}

fn state_metadata_from_state(state: &CacheState) -> StateMetadata {
    StateMetadata {
        schema_version: SCHEMA_VERSION,
        repo_path: state.repo_path.clone(),
        include_tests: state.include_tests,
        language: state.language,
        head: state.head.clone(),
        content_fingerprint: files_fingerprint(&state.files),
        file_count: state.files.len(),
    }
}

fn files_fingerprint(files: &BTreeMap<String, FileArtifact>) -> String {
    let mut hasher = Sha256::new();
    for (path, artifact) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(artifact.source_hash.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    hex_hash(digest.as_slice())
}

fn combined_state_fingerprint(metas: &[StateMetadata]) -> String {
    let mut hasher = Sha256::new();
    for meta in metas {
        hasher.update(meta.language.cache_key().as_bytes());
        hasher.update([0]);
        hasher.update(meta.content_fingerprint.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    hex_hash(digest.as_slice())
}

fn compatible_state_metadata(
    meta: &StateMetadata,
    repo_path: &Path,
    include_tests: bool,
    language: Language,
) -> bool {
    meta.schema_version == SCHEMA_VERSION
        && meta.repo_path == repo_path.display().to_string()
        && meta.include_tests == include_tests
        && meta.language == language
}

fn compatible_query_metadata(
    meta: &QueryCacheMetadata,
    repo_path: &Path,
    include_tests: bool,
) -> bool {
    meta.schema_version == SCHEMA_VERSION
        && meta.repo_path == repo_path.display().to_string()
        && meta.include_tests == include_tests
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

fn parse_name_status_z(output: &[u8]) -> Vec<String> {
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
            changed.push(String::from_utf8_lossy(old_path).into_owned());
            changed.push(String::from_utf8_lossy(new_path).into_owned());
            continue;
        }

        let Some(path) = parts.next() else {
            break;
        };
        changed.push(String::from_utf8_lossy(path).into_owned());
    }

    changed
}

fn collect_repo_status_snapshot(
    repo_path: &Path,
    current_head: &str,
    shared_previous_head: Option<&str>,
    timings: &mut TimingCollector,
) -> anyhow::Result<RepoStatusSnapshot> {
    timings.measure("freshness_git", || -> anyhow::Result<RepoStatusSnapshot> {
        let committed_paths = if let Some(previous_head) = shared_previous_head {
            if previous_head == current_head {
                Vec::new()
            } else {
                parse_name_status_z(&run_git_bytes(
                    repo_path,
                    &[
                        "diff",
                        "--name-status",
                        "-z",
                        previous_head,
                        current_head,
                        "--",
                    ],
                )?)
            }
        } else {
            Vec::new()
        };

        let dirty_paths = parse_name_status_z(&run_git_bytes(
            repo_path,
            &["diff", "--name-status", "-z", "HEAD", "--"],
        )?);
        let untracked_paths = run_git_bytes(
            repo_path,
            &["ls-files", "--others", "--exclude-standard", "-z", "--"],
        )?
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .filter_map(|path| String::from_utf8(path.to_vec()).ok())
        .collect();

        Ok(RepoStatusSnapshot {
            current_head: current_head.to_string(),
            shared_previous_head: shared_previous_head.map(str::to_string),
            committed_paths,
            dirty_paths,
            untracked_paths,
        })
    })
}

impl RepoStatusSnapshot {
    fn changed_files_for(
        &self,
        state_head: &str,
        include_tests: bool,
        language: Language,
    ) -> Option<Vec<String>> {
        let mut changed = Vec::new();

        for path in &self.dirty_paths {
            if is_relevant_path(path, include_tests, language) {
                changed.push(path.clone());
            }
        }
        for path in &self.untracked_paths {
            if is_relevant_path(path, include_tests, language) {
                changed.push(path.clone());
            }
        }

        if state_head == self.current_head {
            changed.sort();
            changed.dedup();
            return Some(changed);
        }

        if self.shared_previous_head.as_deref() != Some(state_head) {
            return None;
        }

        for path in &self.committed_paths {
            if is_relevant_path(path, include_tests, language) {
                changed.push(path.clone());
            }
        }

        changed.sort();
        changed.dedup();
        Some(changed)
    }
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
    current_files: &BTreeMap<String, PathBuf>,
    include_tests: bool,
    language: Language,
) -> anyhow::Result<BTreeMap<String, FileArtifact>> {
    let mut parser = codepaths::new_parser(language)?;
    let mut files = BTreeMap::new();

    for (relative_path, path) in current_files {
        files.insert(
            relative_path.clone(),
            build_file_artifact(relative_path, path, include_tests, language, &mut parser)?,
        );
    }

    Ok(files)
}

fn full_rebuild_language(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
    language: Language,
    current_files: &BTreeMap<String, PathBuf>,
    timings: &mut TimingCollector,
) -> anyhow::Result<LanguageLoadResult> {
    let _progress = DelayedMessage::new(
        format!("Building {} graph...", language.display_name()),
        std::time::Duration::from_secs(1),
    );
    let files = build_all_artifacts(current_files, include_tests, language)?;
    let graph = timings.measure("graph_rebuild", || {
        codepaths::build_graph_from_artifacts(&files)
    });
    let changed_files = files.keys().cloned().collect::<Vec<_>>();

    write_graph(repo_path, include_tests, language, &graph)?;
    let state_meta = write_state(
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
    drop(_progress);

    Ok(LanguageLoadResult {
        graph,
        state_meta,
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
    current_files: &BTreeMap<String, PathBuf>,
    changed_files: &[String],
    timings: &mut TimingCollector,
) -> anyhow::Result<LanguageLoadResult> {
    let mut state = match read_state(repo_path, include_tests, language, timings) {
        Ok(state) => state,
        Err(_) => {
            return full_rebuild_language(
                repo_path,
                include_tests,
                current_head,
                language,
                current_files,
                timings,
            )
        }
    };
    if state.schema_version != SCHEMA_VERSION
        || state.repo_path != repo_path.display().to_string()
        || state.include_tests != include_tests
        || state.language != language
    {
        return full_rebuild_language(
            repo_path,
            include_tests,
            current_head,
            language,
            current_files,
            timings,
        );
    }

    let _progress = DelayedMessage::new(
        format!(
            "Incrementally rebuilding {} graph ({} changed file{})",
            language.display_name(),
            changed_files.len(),
            if changed_files.len() == 1 { "" } else { "s" }
        ),
        std::time::Duration::from_secs(1),
    );

    let mut parser = codepaths::new_parser(language)?;

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

    state.head = current_head.to_string();
    let graph = timings.measure("graph_rebuild", || {
        codepaths::build_graph_from_artifacts(&state.files)
    });
    let state_meta = write_state(repo_path, include_tests, language, &state)?;
    write_graph(repo_path, include_tests, language, &graph)?;
    drop(_progress);

    Ok(LanguageLoadResult {
        graph,
        state_meta,
        outcome: LoadGraphOutcome {
            mode: LoadGraphMode::Incremental,
            changed_files: changed_files.to_vec(),
        },
    })
}

fn read_language_state_metadata(
    repo_path: &Path,
    include_tests: bool,
    timings: &mut TimingCollector,
) -> BTreeMap<Language, Option<StateMetadata>> {
    let mut metas = BTreeMap::new();
    for language in Language::ALL {
        let meta = read_state_metadata(repo_path, include_tests, language, timings).ok();
        metas.insert(language, meta);
    }
    metas
}

fn shared_previous_head(
    metas: &BTreeMap<Language, Option<StateMetadata>>,
    current_head: &str,
) -> Option<String> {
    let distinct_heads = metas
        .values()
        .filter_map(|meta| meta.as_ref())
        .filter(|meta| meta.head != current_head)
        .map(|meta| meta.head.clone())
        .collect::<BTreeSet<_>>();

    if distinct_heads.len() == 1 {
        distinct_heads.into_iter().next()
    } else {
        None
    }
}

fn build_language_plans(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
    current_files_by_language: BTreeMap<Language, BTreeMap<String, PathBuf>>,
    state_metas: BTreeMap<Language, Option<StateMetadata>>,
    snapshot: &RepoStatusSnapshot,
) -> Vec<LanguageCachePlan> {
    let mut plans = Vec::new();

    for language in Language::ALL {
        let current_files = current_files_by_language
            .get(&language)
            .cloned()
            .unwrap_or_default();
        let state_meta = state_metas
            .get(&language)
            .cloned()
            .flatten()
            .filter(|meta| compatible_state_metadata(meta, repo_path, include_tests, language));

        let action = match &state_meta {
            None if current_files.is_empty() => LanguageCacheAction::Skip,
            None => LanguageCacheAction::FullRebuild,
            Some(meta) => match snapshot.changed_files_for(&meta.head, include_tests, language) {
                None => LanguageCacheAction::FullRebuild,
                Some(changed_files) if changed_files.is_empty() => {
                    if meta.head == current_head {
                        LanguageCacheAction::Reuse
                    } else {
                        LanguageCacheAction::RefreshHeadOnly
                    }
                }
                Some(changed_files) => LanguageCacheAction::Incremental { changed_files },
            },
        };

        plans.push(LanguageCachePlan {
            language,
            current_files,
            state_meta,
            action,
        });
    }

    plans
}

fn write_refreshed_metadata_heads(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
    plans: &[LanguageCachePlan],
) -> anyhow::Result<Vec<StateMetadata>> {
    let mut metas = Vec::new();
    for plan in plans {
        match (&plan.action, &plan.state_meta) {
            (LanguageCacheAction::Skip, _) => {}
            (LanguageCacheAction::Reuse, Some(meta)) => metas.push(meta.clone()),
            (LanguageCacheAction::RefreshHeadOnly, Some(meta)) => {
                let mut refreshed = meta.clone();
                refreshed.head = current_head.to_string();
                write_state_metadata(repo_path, include_tests, plan.language, &refreshed)?;
                metas.push(refreshed);
            }
            _ => {}
        }
    }
    metas.sort_by_key(|meta| meta.language);
    Ok(metas)
}

fn query_cache_metadata_from(
    repo_path: &Path,
    include_tests: bool,
    current_head: &str,
    state_fingerprint: String,
) -> QueryCacheMetadata {
    QueryCacheMetadata {
        schema_version: SCHEMA_VERSION,
        repo_path: repo_path.display().to_string(),
        include_tests,
        head: current_head.to_string(),
        state_fingerprint,
    }
}

fn load_or_build_query_cache_impl(
    repo_path: &Path,
    include_tests: bool,
    timings: &mut TimingCollector,
) -> anyhow::Result<LoadQueryResult> {
    let current_head = timings.measure("head_hash", || head_hash(repo_path))?;
    let current_files_by_language = timings.measure("source_scan", || {
        codepaths::collect_relevant_source_files(repo_path, include_tests)
    });
    let state_metas = read_language_state_metadata(repo_path, include_tests, timings);
    let shared_previous_head = shared_previous_head(&state_metas, &current_head);
    let snapshot = collect_repo_status_snapshot(
        repo_path,
        &current_head,
        shared_previous_head.as_deref(),
        timings,
    )?;
    let plans = build_language_plans(
        repo_path,
        include_tests,
        &current_head,
        current_files_by_language,
        state_metas,
        &snapshot,
    );

    let can_reuse_query_cache = plans.iter().all(|plan| {
        matches!(
            plan.action,
            LanguageCacheAction::Skip
                | LanguageCacheAction::Reuse
                | LanguageCacheAction::RefreshHeadOnly
        )
    });

    if can_reuse_query_cache {
        let effective_state_metas =
            write_refreshed_metadata_heads(repo_path, include_tests, &current_head, &plans)?;
        let state_fingerprint = combined_state_fingerprint(&effective_state_metas);
        if let Ok(meta) = read_query_cache_metadata(repo_path, include_tests, timings) {
            if compatible_query_metadata(&meta, repo_path, include_tests)
                && meta.state_fingerprint == state_fingerprint
            {
                if let Ok(payload) = read_query_cache_payload(repo_path, include_tests, timings) {
                    if meta.head != current_head {
                        write_query_cache_metadata(
                            repo_path,
                            include_tests,
                            &query_cache_metadata_from(
                                repo_path,
                                include_tests,
                                &current_head,
                                state_fingerprint,
                            ),
                        )?;
                    }
                    return Ok(LoadQueryResult {
                        payload,
                        outcome: LoadGraphOutcome {
                            mode: LoadGraphMode::Reused,
                            changed_files: Vec::new(),
                        },
                    });
                }
            }
        }
    }

    let mut graphs = Vec::new();
    let mut state_metas = Vec::new();
    let mut changed_files = Vec::new();
    let mut mode = LoadGraphMode::Reused;

    for plan in plans {
        let result = match plan.action {
            LanguageCacheAction::Skip => None,
            LanguageCacheAction::Reuse | LanguageCacheAction::RefreshHeadOnly => {
                match read_graph(repo_path, include_tests, plan.language, timings) {
                    Ok(graph) => {
                        let mut state_meta =
                            plan.state_meta.expect("reused cache missing metadata");
                        if matches!(plan.action, LanguageCacheAction::RefreshHeadOnly) {
                            state_meta.head = current_head.clone();
                            write_state_metadata(
                                repo_path,
                                include_tests,
                                plan.language,
                                &state_meta,
                            )?;
                        }
                        Some(LanguageLoadResult {
                            graph,
                            state_meta,
                            outcome: LoadGraphOutcome {
                                mode: LoadGraphMode::Reused,
                                changed_files: Vec::new(),
                            },
                        })
                    }
                    Err(_) => Some(full_rebuild_language(
                        repo_path,
                        include_tests,
                        &current_head,
                        plan.language,
                        &plan.current_files,
                        timings,
                    )?),
                }
            }
            LanguageCacheAction::Incremental {
                changed_files: planned_changed_files,
            } => Some(incremental_rebuild_language(
                repo_path,
                include_tests,
                &current_head,
                plan.language,
                &plan.current_files,
                &planned_changed_files,
                timings,
            )?),
            LanguageCacheAction::FullRebuild => Some(full_rebuild_language(
                repo_path,
                include_tests,
                &current_head,
                plan.language,
                &plan.current_files,
                timings,
            )?),
        };

        let Some(result) = result else {
            continue;
        };

        if !result.graph.nodes.is_empty()
            || !result.graph.edges.is_empty()
            || !result.graph.references.is_empty()
        {
            graphs.push(result.graph);
        }
        state_metas.push(result.state_meta);
        changed_files.extend(result.outcome.changed_files);
        mode = merge_mode(mode, result.outcome.mode);
    }

    state_metas.sort_by_key(|meta| meta.language);
    changed_files.sort();
    changed_files.dedup();

    let merged_graph = if graphs.is_empty() {
        empty_graph()
    } else {
        timings.measure("merge_graphs", || merge_graphs(&graphs))
    };
    let payload = timings.measure("derived_index_build", || {
        QueryCachePayload::from_graph(merged_graph)
    });
    let state_fingerprint = combined_state_fingerprint(&state_metas);

    write_query_cache_payload(repo_path, include_tests, &payload)?;
    write_query_cache_metadata(
        repo_path,
        include_tests,
        &query_cache_metadata_from(repo_path, include_tests, &current_head, state_fingerprint),
    )?;

    Ok(LoadQueryResult {
        payload,
        outcome: LoadGraphOutcome {
            mode,
            changed_files,
        },
    })
}

pub fn load_or_build_query_cache(
    repo_path: &Path,
    include_tests: bool,
    timings: &mut TimingCollector,
) -> anyhow::Result<LoadQueryResult> {
    load_or_build_query_cache_impl(repo_path, include_tests, timings)
}

pub fn load_or_build_graph(
    repo_path: &Path,
    include_tests: bool,
) -> anyhow::Result<LoadGraphResult> {
    let mut timings = TimingCollector::disabled();
    let result = load_or_build_query_cache_impl(repo_path, include_tests, &mut timings)?;
    Ok(LoadGraphResult {
        graph: result.payload.graph.clone(),
        outcome: result.outcome,
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
