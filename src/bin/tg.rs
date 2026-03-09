use tracegrep::cli::Cli;
use tracegrep::commands;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse()?;

    commands::query::run(commands::query::QueryOptions {
        json_output: cli.json,
        compact: cli.compact,
        repo: &cli.repo,
        search_paths: &cli.search_paths,
        depth: cli.depth,
        include_tests: cli.include_tests,
        include_test_callers: cli.include_test_callers,
        pattern: &cli.pattern,
        rg_args: &cli.rg_args,
    })
}
