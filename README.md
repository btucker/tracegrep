# tracegrep

`tracegrep` layers Rust call-graph context on top of `rg` results.

This repo also includes a Claude Code plugin, hooks, and skill that tell the agent to prefer `tg` over plain grep-style search.

## Usage

```bash
# Search with caller/reference context
tracegrep tool_data --repo /path/to/repo

# Inline context onto the location line
tracegrep --compact tool_data --repo /path/to/repo

# Include test callers when they matter
tracegrep --include-tests --include-test-callers tool_data --repo /path/to/repo
```

## Notes

- This is Rust-only today. The parser is built on `tree-sitter-rust`.
- `rg` must be installed and available on `PATH`.
- The call graph is rebuilt automatically when the cache under `~/.cache/tracegrep/` is missing or its stored `HEAD` no longer matches the repo.
- Function references passed as arguments are shown separately from direct callers.

## Claude Code plugin

The repository now ships a Claude Code plugin under `.claude-plugin/`, hooks under `hooks/`, and a skill under `skills/search-with-call-graph-context/`.

When Claude Code loads this plugin, the `search-with-call-graph-context` skill tells the agent to:

- prefer `tg` instead of `rg` for code search
- use `--json` only when structured output is needed
- fall back to `tracegrep` or `cargo run --` if needed
- use plain `rg` or `grep` only when the user explicitly asks for them or the search target is outside tracegrep's model

The plugin hooks reinforce that behavior by:

- injecting the skill text at session start
- denying Claude Code's built-in `Grep` tool for repository search in this repo
- denying Bash `rg`, `grep`, and `git grep` searches so Claude reruns them with `tg`
