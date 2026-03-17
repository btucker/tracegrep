use std::io::Write;
use std::process::ExitCode;

use tracegrep::cli::Cli;
use tracegrep::commands;
use tracegrep::completions;

fn main() -> ExitCode {
    tracegrep::cli_exit_code(run())
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse()?;

    if let Some(target) = cli.generate.as_deref() {
        let mut stdout = std::io::stdout().lock();
        write!(stdout, "{}", completions::generate(target)?)?;
        Ok(())
    } else if cli.install_completions.is_some() {
        let shell_arg = cli
            .install_completions
            .as_deref()
            .filter(|value| *value != "auto");
        let result = completions::install(shell_arg)?;
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "Installed {:?} completions.", result.shell)?;
        for path in result.written_files {
            writeln!(stdout, "  wrote {}", path.display())?;
        }
        for path in result.updated_rc_files {
            writeln!(stdout, "  updated {}", path.display())?;
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
