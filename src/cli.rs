use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

pub struct Cli {
    pub json: bool,
    pub compact: bool,
    pub repo: String,
    pub search_paths: Vec<String>,
    pub depth: usize,
    pub include_tests: bool,
    pub include_test_callers: bool,
    pub pattern: String,
    pub rg_args: Vec<String>,
}

impl Cli {
    pub fn parse() -> anyhow::Result<Self> {
        let args: Vec<String> = std::env::args().skip(1).collect();

        if args.iter().any(|arg| arg == "-h" || arg == "--help") {
            print!("{}", Self::help_text());
            std::process::exit(0);
        }
        if args.iter().any(|arg| arg == "-V" || arg == "--version") {
            println!("{}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        if args.is_empty() {
            anyhow::bail!("{}", Self::help_text());
        }

        let mut cli = Cli {
            json: false,
            compact: false,
            repo: ".".to_string(),
            search_paths: Vec::new(),
            depth: 1,
            include_tests: false,
            include_test_callers: false,
            pattern: String::new(),
            rg_args: Vec::new(),
        };

        let mut passthrough = Vec::new();
        let mut after_delimiter = false;
        let mut idx = 0;
        while idx < args.len() {
            if !after_delimiter {
                match args[idx].as_str() {
                    "--" => {
                        after_delimiter = true;
                        idx += 1;
                        continue;
                    }
                    "--json" => {
                        cli.json = true;
                        idx += 1;
                        continue;
                    }
                    "--compact" => {
                        cli.compact = true;
                        idx += 1;
                        continue;
                    }
                    "--include-tests" => {
                        cli.include_tests = true;
                        idx += 1;
                        continue;
                    }
                    "--include-test-callers" => {
                        cli.include_test_callers = true;
                        idx += 1;
                        continue;
                    }
                    "--depth" => {
                        idx += 1;
                        let value = args
                            .get(idx)
                            .ok_or_else(|| anyhow::anyhow!("missing value for --depth"))?;
                        cli.depth = value
                            .parse()
                            .map_err(|_| anyhow::anyhow!("invalid value for --depth: {value}"))?;
                        idx += 1;
                        continue;
                    }
                    _ => {}
                }
            }
            passthrough.push(args[idx].clone());
            idx += 1;
        }

        if cli.json && cli.compact {
            anyhow::bail!("--compact cannot be used together with --json");
        }

        let ParsedPassthrough {
            pattern,
            search_paths,
            rg_args,
        } = parse_passthrough(&passthrough, after_delimiter)?;
        cli.pattern = pattern;
        cli.rg_args = rg_args;

        if !search_paths.is_empty() {
            let (repo, relative_paths) = infer_multi_path_repo_and_paths(&search_paths)?;
            cli.repo = repo;
            cli.search_paths = relative_paths;
        }

        Ok(cli)
    }

    fn help_text() -> String {
        format!(
            "tracegrep {}\n\n\
Search code with ripgrep and language-aware call graph context.\n\n\
Usage:\n  tracegrep [flags/rg flags] <pattern> [path ...]\n\n\
Supported files: .rs, .py, .js, .jsx, .ts, .tsx\n\n\
Tracegrep flags:\n  --json                  Output enriched results as JSON\n  --compact               Collapse human-readable context onto the location line\n  --depth <N>             How many caller levels to show (default: 1)\n  --include-tests         Include test-file callers and references in the graph\n  --include-test-callers  Show callers that originate from test code\n  -h, --help              Print help\n  -V, --version           Print version\n\n\
Any unrecognized flags before <pattern> are forwarded to rg.\n\
Positional [path ...] arguments use rg semantics.\n",
            env!("CARGO_PKG_VERSION")
        )
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
