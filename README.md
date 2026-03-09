# tracegrep

`tracegrep` layers call-graph context on top of `rg` results.

It exists to give coding agent instant context for how a line of code is used in the codebase. This allows the coding agent to gain a more complete understanding prior to making changes.

This repo also includes a Claude Code plugin, hooks, and skill that add tracegrep call-graph context to Claude's normal search flow.

## Usage

```bash
# Search with caller/reference context
tracegrep tool_data /path/to/repo

# Inline context onto the location line
tracegrep --compact tool_data /path/to/repo

# Include test callers when they matter
tracegrep --include-tests --include-test-callers tool_data /path/to/repo

# Or keep using the older explicit form
tracegrep --repo /path/to/repo tool_data
```

## Example output

Searching for `validate_body` in a small Rust repo shows the difference in
shape immediately:

<table>
  <tr>
    <th><code>rg -n validate_body</code></th>
    <th><code>tracegrep validate_body /path/to/repo</code></th>
  </tr>
  <tr>
    <td valign="top">
      <pre lang="text"><code>src/main.rs:3:    register_handler(validate_body);
src/main.rs:8:        validate_body();
src/main.rs:13:fn validate_body() {
tests/integration.rs:1:fn test_validate_body() {
tests/integration.rs:2:    validate_body();</code></pre>
    </td>
    <td valign="top">
      <pre lang="text"><code>src/main.rs:main:3
      register_handler(validate_body);

src/main.rs:router:8
          validate_body();
  Called via:
    src/main.rs:main:1

src/main.rs:validate_body:13
  fn validate_body() {
  Called via:
    src/main.rs:router:6  (when method == "POST")
  Referenced by:
    src/main.rs:main:1  (passed to register_handler)

tests/integration.rs:1:fn test_validate_body() {
tests/integration.rs:2:    validate_body();</code></pre>
    </td>
  </tr>
</table>

## Notes

- This is Rust-only today. The parser is built on `tree-sitter-rust`.
- `rg` must be installed and available on `PATH`.
- The preferred CLI shape mirrors `rg`: `tracegrep [flags] <pattern> [path]`.
- `--repo` still works as an explicit compatibility flag and can be combined
  with a positional path to search within a subdirectory of that repo.
- Most `rg` flags can be passed through before `<pattern>`, but tools that
  expect raw `rg` output should keep using `rg`.
- The call graph is rebuilt automatically when the cache under `~/.cache/tracegrep/` is missing or its stored `HEAD` no longer matches the repo.
- Function references passed as arguments are shown separately from direct callers.

## Claude Code plugin

The repository now ships a Claude Code plugin under `.claude-plugin/`, hooks under `hooks/`, and a skill under `skills/search-with-call-graph-context/`.

When Claude Code loads this plugin, the `search-with-call-graph-context` skill tells the agent to:

- prefer tracegrep-aware search instead of raw shell `rg`
- use `--json` only when structured output is needed
- fall back to `tracegrep` or `cargo run --` if needed
- use plain `rg` or `grep` only when the user explicitly asks for them or the search target is outside tracegrep's model

The plugin hooks reinforce that behavior by:

- injecting the skill text at session start
- letting Claude Code's built-in `Grep` tool run normally
- rerunning `Grep` queries through `tg` and attaching the full annotated output through a `PostToolUse` hook
- denying Bash `rg`, `grep`, and `git grep` searches so Claude reruns them with a tracegrep-aware path
