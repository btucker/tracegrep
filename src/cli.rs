use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "tracegrep",
    version,
    about = "Search code with ripgrep and language-aware call graph context.",
    disable_help_subcommand = true,
    trailing_var_arg = true,
    override_usage = "tracegrep [OPTIONS/rg flags] <pattern> [path ...]\n       tracegrep --build-index [path ...]\n       tracegrep --generate <TARGET>\n       tracegrep --install-completions [SHELL]",
    after_help = "Any unrecognized flags before <pattern> are forwarded to rg.\nPositional [path ...] arguments use rg semantics."
)]
struct RawCli {
    /// Build or refresh the cached call graph index, then exit.
    #[arg(long)]
    build_index: bool,
    /// Generate shell completions: complete-bash, complete-zsh, or complete-fish.
    #[arg(long)]
    generate: Option<String>,
    /// Install shell completions for the current or given shell.
    #[arg(long, num_args = 0..=1, default_missing_value = "auto")]
    install_completions: Option<String>,
    /// Output enriched results as JSON.
    #[arg(long)]
    json: bool,
    /// Collapse human-readable context onto the location line.
    #[arg(long)]
    compact: bool,
    /// Include test-file callers and references in the graph.
    #[arg(long)]
    include_tests: bool,
    /// Show callers that originate from test code.
    #[arg(long)]
    include_test_callers: bool,
    /// How many caller levels to show.
    #[arg(long, default_value_t = 1)]
    depth: usize,
    /// Max callers or references to show per section.
    #[arg(long, default_value_t = 5)]
    max_context: usize,
    #[arg(allow_hyphen_values = true, value_name = "ARG")]
    raw_args: Vec<String>,
}

pub struct Cli {
    pub build_index: bool,
    pub generate: Option<String>,
    pub install_completions: Option<String>,
    pub json: bool,
    pub compact: bool,
    pub repo: String,
    pub search_paths: Vec<String>,
    pub depth: usize,
    pub max_context: usize,
    pub include_tests: bool,
    pub include_test_callers: bool,
    pub pattern: String,
    pub rg_args: Vec<String>,
}

impl Cli {
    pub fn parse() -> anyhow::Result<Self> {
        let original_args: Vec<String> = std::env::args().collect();
        let allow_dash_pattern = original_args.iter().any(|arg| arg == "--");
        let raw = RawCli::parse();

        let mut cli = Cli {
            build_index: raw.build_index,
            generate: raw.generate,
            install_completions: raw.install_completions,
            json: raw.json,
            compact: raw.compact,
            repo: ".".to_string(),
            search_paths: Vec::new(),
            depth: raw.depth,
            max_context: raw.max_context,
            include_tests: raw.include_tests,
            include_test_callers: raw.include_test_callers,
            pattern: String::new(),
            rg_args: Vec::new(),
        };

        if cli.json && cli.compact {
            anyhow::bail!("--compact cannot be used together with --json");
        }
        if cli.build_index && (cli.json || cli.compact || cli.include_test_callers) {
            anyhow::bail!(
                "--build-index cannot be used with --json, --compact, or --include-test-callers"
            );
        }
        if cli.generate.is_some() && cli.install_completions.is_some() {
            anyhow::bail!("--generate cannot be used together with --install-completions");
        }

        if cli.generate.is_some() {
            if cli.json
                || cli.compact
                || cli.include_tests
                || cli.include_test_callers
                || cli.build_index
                || !raw.raw_args.is_empty()
            {
                anyhow::bail!(
                    "--generate cannot be combined with query flags or positional arguments"
                );
            }
            return Ok(cli);
        }

        if cli.install_completions.is_some() {
            if cli.json
                || cli.compact
                || cli.include_tests
                || cli.include_test_callers
                || cli.build_index
                || !raw.raw_args.is_empty()
            {
                anyhow::bail!(
                    "--install-completions cannot be combined with query flags or positional arguments"
                );
            }
            return Ok(cli);
        }

        if cli.build_index {
            if raw.raw_args.iter().any(|arg| arg.starts_with('-')) {
                anyhow::bail!("--build-index accepts only optional path arguments");
            }
            if !raw.raw_args.is_empty() {
                let (repo, _relative_paths) = infer_multi_path_repo_and_paths(&raw.raw_args)?;
                cli.repo = repo;
            }
            return Ok(cli);
        }

        let ParsedPassthrough {
            pattern,
            search_paths,
            rg_args,
        } = parse_passthrough(&raw.raw_args, allow_dash_pattern)?;
        cli.pattern = pattern;
        cli.rg_args = rg_args;

        if !search_paths.is_empty() {
            let (repo, relative_paths) = infer_multi_path_repo_and_paths(&search_paths)?;
            cli.repo = repo;
            cli.search_paths = relative_paths;
        }

        Ok(cli)
    }
}

struct ParsedPassthrough {
    pattern: String,
    search_paths: Vec<String>,
    rg_args: Vec<String>,
}

fn parse_passthrough(
    args: &[String],
    allow_dash_pattern: bool,
) -> anyhow::Result<ParsedPassthrough> {
    if args.is_empty() {
        anyhow::bail!("missing pattern");
    }

    let trailing_paths = args
        .iter()
        .rev()
        .take_while(|arg| looks_like_path(arg))
        .count()
        .min(args.len().saturating_sub(1));

    let pattern_idx = args.len() - trailing_paths - 1;
    let search_paths = args[pattern_idx + 1..].to_vec();

    let pattern = args[pattern_idx].clone();
    if pattern.starts_with('-') && !allow_dash_pattern {
        anyhow::bail!("missing pattern (use -- before patterns that start with -)");
    }

    Ok(ParsedPassthrough {
        pattern,
        search_paths,
        rg_args: args[..pattern_idx].to_vec(),
    })
}

fn looks_like_path(value: &str) -> bool {
    let path = Path::new(value);
    path.exists() || value == "." || value == ".." || value.contains(std::path::MAIN_SEPARATOR)
}

fn infer_multi_path_repo_and_paths(
    search_paths: &[String],
) -> anyhow::Result<(String, Vec<String>)> {
    let cwd = std::env::current_dir()?;
    let mut resolved_targets = Vec::with_capacity(search_paths.len());
    let mut bases = Vec::with_capacity(search_paths.len());
    let mut git_roots = Vec::with_capacity(search_paths.len());

    for path in search_paths {
        let resolved = cwd
            .join(path)
            .canonicalize()
            .map_err(|error| anyhow::anyhow!("failed to resolve search path {path:?}: {error}"))?;
        let base = if resolved.is_file() {
            resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        } else {
            resolved.clone()
        };
        resolved_targets.push(resolved);
        git_roots.push(git_top_level(&base)?);
        bases.push(base);
    }

    let repo_path = infer_repo_root(&git_roots, &bases)?;
    let relative_paths = resolved_targets
        .into_iter()
        .map(|path| relativize_to_repo(&repo_path, &path))
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok((repo_path.to_string_lossy().into_owned(), relative_paths))
}

fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut ancestor = paths.first()?.clone();
    for path in &paths[1..] {
        while !path.starts_with(&ancestor) {
            ancestor = ancestor.parent()?.to_path_buf();
        }
    }
    Some(ancestor)
}

fn infer_repo_root(git_roots: &[Option<PathBuf>], bases: &[PathBuf]) -> anyhow::Result<PathBuf> {
    let mut distinct_git_roots = git_roots.iter().flatten();
    if let Some(first) = distinct_git_roots.next() {
        if distinct_git_roots.all(|root| root == first) {
            return Ok(first.clone());
        }
        anyhow::bail!("positional paths must belong to the same git repository");
    }

    common_ancestor(bases)
        .ok_or_else(|| anyhow::anyhow!("failed to determine a common root for positional paths"))
}

fn git_top_level(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }

    let root = String::from_utf8(output.stdout)?.trim().to_string();
    if root.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PathBuf::from(root)))
    }
}

fn relativize_to_repo(repo_path: &Path, target: &Path) -> anyhow::Result<String> {
    let relative = target.strip_prefix(repo_path).map_err(|_| {
        anyhow::anyhow!(
            "search path {} is outside inferred root {}",
            target.display(),
            repo_path.display()
        )
    })?;

    if relative.as_os_str().is_empty() {
        Ok(".".to_string())
    } else {
        Ok(relative.to_string_lossy().into_owned())
    }
}
