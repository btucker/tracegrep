---
name: search-with-call-graph-context
description: Use when searching Rust code, tracing references or callers, or when you would normally reach for `rg` or `grep` in a repository that has `tg` available. Prefer tracegrep-aware search over raw `rg` or `grep`, and use `--json` only when structured output is needed.
---

# Using tracegrep

Prefer tracegrep-aware search over raw `rg`, `grep`, or other plain-text search tools when you need to search code in this repository.

## Claude Code behavior

In Claude Code, treat this skill as an augmentation of normal repository search behavior:

- Built-in `Grep` is allowed in this repo.
- A `PostToolUse` hook may rerun the same search with `tg` and attach the full annotated output after a `Grep` search.
- Do not run Bash `rg`, `grep`, or `git grep` commands when built-in `Grep` or `tg` can answer the question.

## Default workflow

1. Start with Claude Code `Grep` or `tg '<pattern>'`.
2. Add repo scope or passthrough flags as needed, for example `tg --repo . '<pattern>'`.
3. Switch to `tg --json` only when you need to parse, filter, or post-process results programmatically.

## Why

`tg` preserves normal search matches while adding Rust call-graph context. That usually makes follow-up inspection faster than a plain `rg` hit list.

## Fallbacks

- If `tg` is not on `PATH`, use `tracegrep`.
- If the binary is not installed but you are in the source repo, use `cargo run --`.
- Use `--json` with those fallbacks only when structured output is needed.
- Only fall back to `rg` or `grep` if the user explicitly asks for them, or if you are searching content that `tracegrep` does not model well.

## Examples

```bash
tg 'tool_data'
tg --repo . 'QueryOptions'
tg --json --include-tests --depth 2 'parse'
```
