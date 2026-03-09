pub struct Cli {
    pub json: bool,
    pub compact: bool,
    pub repo: String,
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

        let pattern = args
            .last()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing pattern"))?;
        let mut cli = Cli {
            json: false,
            compact: false,
            repo: ".".to_string(),
            depth: 1,
            include_tests: false,
            include_test_callers: false,
            pattern,
            rg_args: Vec::new(),
        };

        let mut idx = 0;
        while idx + 1 < args.len() {
            match args[idx].as_str() {
                "--json" => cli.json = true,
                "--compact" => cli.compact = true,
                "--include-tests" => cli.include_tests = true,
                "--include-test-callers" => cli.include_test_callers = true,
                "--repo" | "-r" => {
                    idx += 1;
                    let value = args
                        .get(idx)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --repo"))?;
                    cli.repo = value.clone();
                }
                "--depth" => {
                    idx += 1;
                    let value = args
                        .get(idx)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --depth"))?;
                    cli.depth = value
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid value for --depth: {value}"))?;
                }
                arg => cli.rg_args.push(arg.to_string()),
            }
            idx += 1;
        }

        if cli.json && cli.compact {
            anyhow::bail!("--compact cannot be used together with --json");
        }

        Ok(cli)
    }

    fn help_text() -> String {
        format!(
            "tracegrep {}\n\n\
Search code with ripgrep and Rust call graph context.\n\n\
Usage:\n  tracegrep [flags/rg flags] <pattern>\n\n\
Tracegrep flags:\n  --json                  Output enriched results as JSON\n  --compact               Collapse human-readable context onto the location line\n  -r, --repo <PATH>       Path to the repository (default: current directory)\n  --depth <N>             How many caller levels to show (default: 1)\n  --include-tests         Include test files and #[cfg(test)] functions in the graph\n  --include-test-callers  Show callers that originate from test code\n  -h, --help              Print help\n  -V, --version           Print version\n\n\
Any unrecognized flags before <pattern> are forwarded to rg.\n",
            env!("CARGO_PKG_VERSION")
        )
    }
}
