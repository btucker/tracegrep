# tracegrep

`tracegrep` layers Rust call-graph context on top of `rg` results.

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
