use tracegrep::cli::Cli;
use tracegrep::commands;
use tracegrep::completions;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse()?;

    if let Some(target) = cli.generate.as_deref() {
        print!("{}", completions::generate(target)?);
        Ok(())
    } else if cli.install_completions.is_some() {
        let shell_arg = cli
            .install_completions
            .as_deref()
            .filter(|value| *value != "auto");
        let result = completions::install(shell_arg)?;
        println!("Installed {:?} completions.", result.shell);
        for path in result.written_files {
            println!("  wrote {}", path.display());
        }
        for path in result.updated_rc_files {
            println!("  updated {}", path.display());
        }
        Ok(())
    } else if cli.build_index {
        commands::build_index::run(commands::build_index::BuildIndexOptions {
            repo: &cli.repo,
            include_tests: cli.include_tests,
        })
    } else {
        commands::query::run(commands::query::QueryOptions {
            json_output: cli.json,
            compact: cli.compact,
            repo: &cli.repo,
            search_paths: &cli.search_paths,
            depth: cli.depth,
            max_context: cli.max_context,
            include_tests: cli.include_tests,
            include_test_callers: cli.include_test_callers,
            pattern: &cli.pattern,
            rg_args: &cli.rg_args,
        })
    }
}
