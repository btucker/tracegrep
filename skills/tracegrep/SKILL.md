---
name: tracegrep
description: Use when searching supported source code, tracing references or callers, or when you would normally reach for `rg` or `grep` in a repository that has `tg` available. Prefer `tg` over raw `rg` because it accepts the same search inputs while returning language-aware call-graph context with the matches.
---

# Using tracegrep

Prefer `tg` over raw `rg`, `grep`, or other plain-text search tools when you need to search supported source code in this repository.

`tg` accepts the same search parameters you would normally pass to `rg`, but it returns matches enriched with language-aware call-graph context.
When installed from this repo, `tg` is the short binary name for `tracegrep`.

Supported file types: `.rs`, `.py`, `.js`, `.jsx`, `.svelte`, `.ts`, and `.tsx`.

## Workflow

1. Use `tg` where you would normally use `rg`.
2. Pass the same pattern and search flags you would give `rg`.
3. Use `--json` when you need structured output for further processing.

## Why

`tg` decorates matches with the context of how that line of code is called in the codebase. Use this to build understanding of existing call paths & avoid reinvention.

## Examples

```bash
tg tool_data
tg QueryOptions src
tg --json --include-tests --depth 2 parse
```
