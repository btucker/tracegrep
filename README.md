# tracegrep

`tracegrep` layers backtrace context on top of `rg` ([ripgrep](https://github.com/BurntSushi/ripgrep)) results.
It exists to give coding agents instant context for how a line of code is used in the codebase. This allows the agent to gain a more complete understanding prior to making changes.
Rust, Python, TypeScript, and JavaScript are currently supported via [treesitter](https://github.com/tree-sitter/tree-sitter).

This repo includes a `SKILL.md` plus plugin wrappers for Claude Code, Codex, and Cursor.

`tracegrep` maintains a mostly compatible CLI to `ripgrep`.

`$ rg tool_block`
```rust
212:    let blocks = extract_tool_blocks(session_events);
279:fn extract_tool_blocks(events: &[StreamEvent]) -> Vec<ToolBlock> {
```

`$ tg tool_block`
```rust
src/daemon/stream.rs:append_tool_data_effects
212:    let blocks = extract_tool_blocks(session_events);
  Called by:
    src/daemon/stream.rs:process_lead_output:110  (when events.get(main_lead_session_name) is Some(lead_events) && ...)

src/daemon/stream.rs:extract_tool_blocks
279:fn extract_tool_blocks(events: &[StreamEvent]) -> Vec<ToolBlock> {
  Called by:
    src/daemon/stream.rs:append_tool_data_effects:206
    src/daemon/stream.rs:process_agent_output:552  (when events.get(name.as_str()) is Some(coworker_events))
```

## Installation

Note: installation differs by environment. The CLI installs with Cargo. Claude Code can also load the packaged skill via the repo's plugin metadata.

### CLI (quick install)

```bash
curl -fsSL https://raw.githubusercontent.com/btucker/tracegrep/main/install.sh | sh
```

This downloads the latest release binary for your platform and installs it to `~/.local/bin`. You can customize the install directory:

```bash
curl -fsSL https://raw.githubusercontent.com/btucker/tracegrep/main/install.sh | INSTALL_DIR=/usr/local/bin sh
```

Or install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/btucker/tracegrep/main/install.sh | VERSION=v0.1.0 sh
```

### CLI (from source)

Install `rg` first, then install `tracegrep` from this repo:

```bash
cargo install --path .
```

This installs both `tracegrep` and `tg`.

Then install shell completions:

```bash
tg --install-completions
```

`tg` auto-detects `bash`, `zsh`, or `fish` from `$SHELL`. For `bash` and `zsh`,
it also updates your shell rc file so completions load in new shells. Restart the
shell after installation, or source the updated rc file once.

### Claude Code (via Plugin Marketplace)

In Claude Code, register the repository marketplace first:

```text
/plugin marketplace add btucker/tracegrep
```

Then install the plugin from that marketplace:

```text
/plugin install tracegrep@tracegrep-dev
```

### Codex and other agents

Use the CLI directly and point the agent at the local skill file:

```text
skills/tracegrep/SKILL.md
```

That keeps the setup explicit: the agent gets the search workflow from the skill, and `tg` provides the actual search behavior.

### Verify installation

Verify the CLI:

```bash
tracegrep --version
tg --help
tg --generate complete-zsh | head
```

In Claude Code, start a fresh session and ask it to search the repo. It should prefer `tg`/`tracegrep`-aware search flow rather than raw `rg`.

## Usage

After `cargo install --path .`, both `tracegrep` and `tg` are installed.

```bash
# Install completions once for the current shell
tg --install-completions

# Search with caller/reference context
tg tool_data /path/to/repo

# Inline context onto the location line
tg --compact tool_data /path/to/repo

# Include test callers when they matter
tg --include-tests --include-test-callers tool_data /path/to/repo

# Search a subset of the repo with rg-style path arguments
tg tool_data src tests

# Emit a completion script without installing it
tg --generate complete-zsh
```

## Example output

Searching for `validate_body` in a small repo shows the difference in
shape immediately:

<table>
  <tr>
    <th><code>rg -n validate_body</code></th>
    <th><code>tg validate_body /path/to/repo</code></th>
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
  Called by:
    src/main.rs:main:1

src/main.rs:validate_body:13
  fn validate_body() {
  Called by:
    src/main.rs:router:6  (when method == "POST")
  Referenced by:
    src/main.rs:main:1  (passed to register_handler)

tests/integration.rs:1:fn test_validate_body() {
tests/integration.rs:2:    validate_body();</code></pre>
    </td>
  </tr>
</table>

## Notes

- Supported source files: `.rs`, `.py`, `.js`, `.jsx`, `.svelte`, `.ts`, and `.tsx`.
- `rg` must be installed and available on `PATH`.
- The preferred CLI shape mirrors `rg`: `tracegrep [flags] <pattern> [path ...]`.
- Most `rg` flags can be passed through before `<pattern>`, but tools that
  expect raw `rg` output should keep using `rg`.
- Each supported language is cached separately under `.git/tracegrep/` by default, then merged in memory at query time.
- Set `TRACEGREP_CACHE_DIR` to override the cache root directory.
- The call graph is rebuilt automatically when the relevant per-language cache is missing or its stored `HEAD` no longer matches the repo.
- Function references passed as arguments are shown separately from direct callers.
- The current resolver is heuristic and language-local; it does not attempt import-aware or type-aware cross-file analysis.

## Claude Code skill

The repository ships a Claude Code skill under `skills/tracegrep/`.

The `tracegrep` skill tells the agent to:

- prefer tracegrep-aware search instead of raw shell `rg`
- use `--json` only when structured output is needed
- fall back to `tracegrep` or `cargo run --` if needed
- use plain `rg` or `grep` only when the user explicitly asks for them or the search target is outside tracegrep's model
