# Eval Harness

This directory contains a small benchmark harness for comparing `codex` and `claude` CLI runs with and without `tg`.

## What it does

- keeps a manifest of historical issue/PR benchmark tasks
- clones the upstream repo into `eval/workspaces/cache/`
- creates detached pre-fix git worktrees for `control` and `tg` runs
- writes redacted task prompts that hide the accepted PR
- generates launcher scripts for both `codex` and `claude`

## Usage

List tasks:

```bash
uv run eval/benchmark.py list
```

Inspect one task:

```bash
uv run eval/benchmark.py show storybook-hide-toolbar-docs
```

Prepare one task:

```bash
uv run eval/benchmark.py prepare storybook-hide-toolbar-docs
```

Launch a prepared run with Codex:

```bash
uv run eval/benchmark.py launch storybook-hide-toolbar-docs --agent codex --condition tg
```

Launch a prepared run with Claude:

```bash
uv run eval/benchmark.py launch storybook-hide-toolbar-docs --agent claude --condition control
```

Pass extra flags to the underlying CLI after `--`:

```bash
uv run eval/benchmark.py launch vite-import-meta-glob-base --agent codex --condition tg -- --model gpt-5
uv run eval/benchmark.py launch vite-import-meta-glob-base --agent claude --condition tg -- --model sonnet --permission-mode acceptEdits
```

## Output layout

Preparing a task creates:

- `eval/workspaces/cache/<repo>/` shared upstream clone
- `eval/workspaces/runs/<task>/worktrees/<condition>/` detached repo snapshot
- `eval/workspaces/runs/<task>/prompts/<condition>.md` agent prompt
- `eval/workspaces/runs/<task>/launch_<agent>_<condition>.sh` launcher script
- `eval/workspaces/runs/<task>/hidden/ground_truth.json` accepted PR metadata for evaluation

## Notes

- The prompts tell agents not to browse the web or inspect PR history.
- The `control` prompt explicitly forbids `tg`.
- The `tg` prompt explicitly prefers `tg`/`tracegrep` for supported source files.
- For tasks where the original issue contained solution leakage, the manifest uses a redacted benchmark prompt instead.
