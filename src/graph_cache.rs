use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::analysis::codepaths::{self, CallGraph};

const CACHE_DIR: &str = ".tracegrep";

#[derive(Serialize, Deserialize)]
struct CacheMetadata {
    head: String,
}

pub fn graph_cache_path(repo_path: &Path, include_tests: bool) -> PathBuf {
    let cache_dir = repo_path.join(CACHE_DIR);
    if include_tests {
        cache_dir.join("codepaths.v2.graph.with-tests")
    } else {
        cache_dir.join("codepaths.v2.graph")
    }
}

fn metadata_path(repo_path: &Path, include_tests: bool) -> PathBuf {
    if include_tests {
        repo_path
            .join(CACHE_DIR)
            .join("codepaths.v2.graph.with-tests.meta.json")
    } else {
        repo_path
            .join(CACHE_DIR)
            .join("codepaths.v2.graph.meta.json")
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

fn read_metadata(repo_path: &Path, include_tests: bool) -> anyhow::Result<Option<CacheMetadata>> {
    let path = metadata_path(repo_path, include_tests);
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&data)?))
}

fn write_metadata(
    repo_path: &Path,
    include_tests: bool,
    metadata: &CacheMetadata,
) -> anyhow::Result<()> {
    let path = metadata_path(repo_path, include_tests);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string(metadata)?)?;
    Ok(())
}

pub fn load_or_build_graph(repo_path: &Path, include_tests: bool) -> anyhow::Result<CallGraph> {
    let graph_path = graph_cache_path(repo_path, include_tests);
    let current_head = head_hash(repo_path)?;

    let cache_is_fresh = graph_path.exists()
        && read_metadata(repo_path, include_tests)?
            .map(|metadata| metadata.head == current_head)
            .unwrap_or(false);

    if cache_is_fresh {
        let data = std::fs::read_to_string(graph_path)?;
        return Ok(serde_json::from_str(&data)?);
    }

    eprintln!("Building call graph...");
    let (_result, call_graph) = codepaths::analyze_and_build_graph(repo_path, include_tests)?;
    eprintln!(
        "Call graph: {} nodes, {} edges, {} references",
        call_graph.nodes.len(),
        call_graph.edges.len(),
        call_graph.references.len()
    );

    if let Some(parent) = graph_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&graph_path, serde_json::to_string(&call_graph)?)?;
    write_metadata(
        repo_path,
        include_tests,
        &CacheMetadata { head: current_head },
    )?;
    eprintln!("Call graph cached to {}", graph_path.display());
    Ok(call_graph)
}
