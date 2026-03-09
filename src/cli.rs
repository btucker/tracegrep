use clap::Parser;

#[derive(Parser)]
#[command(
    name = "tracegrep",
    about = "Search code with ripgrep and Rust call graph context",
    long_about = "tracegrep layers a Rust function call graph on top of ripgrep results.\n\
        It automatically builds and caches a graph for a repository, then enriches\n\
        each rg match with caller and function-reference context. Graphs are cached\n\
        in .tracegrep/ and refreshed when HEAD changes.",
    version
)]
pub struct Cli {
    /// Output enriched results as JSON (one JSON object per match line)
    #[arg(long)]
    pub json: bool,

    /// Collapse human-readable context onto the location line
    #[arg(long, conflicts_with = "json")]
    pub compact: bool,

    /// Path to the repository (default: current directory)
    #[arg(short, long, default_value = ".")]
    pub repo: String,

    /// How many caller levels to show (default: 1)
    #[arg(long, default_value = "1")]
    pub depth: usize,

    /// Include test files and #[cfg(test)] functions in the graph
    #[arg(long)]
    pub include_tests: bool,

    /// Show callers that originate from test code
    #[arg(long)]
    pub include_test_callers: bool,

    /// The search pattern passed to rg
    #[arg(required = true)]
    pub pattern: String,

    /// Additional arguments passed through to rg
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub rg_args: Vec<String>,
}
