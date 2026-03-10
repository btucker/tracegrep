use std::path::Path;

use crate::graph_cache::{load_or_build_query_cache, LoadGraphMode};
use crate::timing::TimingCollector;

pub struct BuildIndexOptions<'a> {
    pub repo: &'a str,
    pub include_tests: bool,
}

pub fn run(options: BuildIndexOptions<'_>) -> anyhow::Result<()> {
    let repo_path = Path::new(options.repo).canonicalize()?;
    let mut timings = TimingCollector::from_env();
    let result = load_or_build_query_cache(&repo_path, options.include_tests, &mut timings)?;
    let message = match result.outcome.mode {
        LoadGraphMode::FullRebuild => "Built index",
        LoadGraphMode::Incremental => "Updated index",
        LoadGraphMode::Reused => "Index already up to date",
    };
    println!("{message} for {}", repo_path.display());
    timings.print("build-index");
    Ok(())
}
