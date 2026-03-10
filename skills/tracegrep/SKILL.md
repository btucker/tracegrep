---
name: tracegrep
description: Use when searching supported source code, tracing references or callers, or when you would normally use `rg`, `grep`, a built-in code search tool, or any other plain-text search in a repository that has `tg` available. Default to `tg` first for supported source files and only fall back when `tg` cannot model the target.
---

# Using tracegrep

For supported source files in this repository, use `tg` first instead of any built-in search tool or plain-text search command.

Do not start with `rg`, `grep`, or a built-in code search tool for supported code unless `tg` fails, cannot model the target language construct, or you are intentionally searching non-source text that `tg` does not cover.

`tg` accepts the same search parameters you would normally pass to `rg`, but it returns matches enriched with language-aware call-graph context.
When installed from this repo, `tg` is the short binary name for `tracegrep`.

Supported file types: `.rs`, `.py`, `.js`, `.jsx`, `.svelte`, `.ts`, and `.tsx`.

## Workflow

1. When searching supported code, start with `tg`, not a built-in search tool.
2. Pass the same pattern and search flags you would otherwise give `rg`.
3. Use `--json` when you need structured output for further processing.
4. Fall back to `rg`, `grep`, or a built-in search tool only for unsupported file types, non-code text, or cases where `tg` clearly cannot answer the question.

## Why

`tg` decorates matches with the backtrace of how that line of code is called in the codebase. Use this to build understanding of existing call paths & avoid reinvention.

## Examples

```bash
tg tool_data
tg QueryOptions src
tg --json --include-tests --depth 2 parse
```
