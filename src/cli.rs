use std::path::Path;

pub struct Cli {
    pub json: bool,
    pub compact: bool,
    pub repo: String,
    pub search_path: Option<String>,
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
            search_path: None,
            depth: 1,
            include_tests: false,
            include_test_callers: false,
            pattern: String::new(),
            rg_args: Vec::new(),
        };

        let mut passthrough = Vec::new();
        let mut repo_override: Option<String> = None;
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
                    "--repo" | "-r" => {
                        idx += 1;
                        let value = args
                            .get(idx)
                            .ok_or_else(|| anyhow::anyhow!("missing value for --repo"))?;
                        repo_override = Some(value.clone());
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
            search_path,
            rg_args,
        } = parse_passthrough(&passthrough, after_delimiter)?;
        cli.pattern = pattern;
        cli.rg_args = rg_args;

        if let Some(repo) = repo_override {
            cli.repo = repo;
            cli.search_path = search_path;
        } else if let Some(path) = search_path {
            let path_buf = Path::new(&path);
            if path_buf.is_file() {
                cli.repo = path_buf
                    .parent()
                    .map(|parent| parent.to_string_lossy().into_owned())
                    .filter(|parent| !parent.is_empty())
                    .unwrap_or_else(|| ".".to_string());
                cli.search_path = path_buf
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned());
            } else {
                cli.repo = path;
            }
        }

        Ok(cli)
    }

    fn help_text() -> String {
        format!(
            "tracegrep {}\n\n\
Search code with ripgrep and Rust call graph context.\n\n\
Usage:\n  tracegrep [flags/rg flags] <pattern> [path]\n\n\
Tracegrep flags:\n  --json                  Output enriched results as JSON\n  --compact               Collapse human-readable context onto the location line\n  -r, --repo <PATH>       Search root override (same role as positional [path])\n  --depth <N>             How many caller levels to show (default: 1)\n  --include-tests         Include test files and #[cfg(test)] functions in the graph\n  --include-test-callers  Show callers that originate from test code\n  -h, --help              Print help\n  -V, --version           Print version\n\n\
Any unrecognized flags before <pattern> are forwarded to rg.\n\
Only a single positional [path] is supported.\n",
            env!("CARGO_PKG_VERSION")
        )
    }
}

struct ParsedPassthrough {
    pattern: String,
    search_path: Option<String>,
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
        .count();
    if trailing_paths > 1 {
        anyhow::bail!("multiple positional paths are not supported; use a single [path] or --repo");
    }

    let (pattern_idx, search_path) = if trailing_paths == 1 && args.len() >= 2 {
        (args.len() - 2, Some(args[args.len() - 1].clone()))
    } else {
        (args.len() - 1, None)
    };

    let pattern = args[pattern_idx].clone();
    if pattern.starts_with('-') && !allow_dash_pattern {
        anyhow::bail!("missing pattern (use -- before patterns that start with -)");
    }

    Ok(ParsedPassthrough {
        pattern,
        search_path,
        rg_args: args[..pattern_idx].to_vec(),
    })
}

fn looks_like_path(value: &str) -> bool {
    let path = Path::new(value);
    path.exists() || value == "." || value == ".." || value.contains(std::path::MAIN_SEPARATOR)
}
