# Eval Harness

This directory contains a small benchmark harness for comparing `codex` and `claude` CLI runs with and without agent access to `tg`.

The claim this harness is built to test is:

- does having the agent use `tg` make a difference in the code it produces?

## What it does

- keeps a manifest of historical issue/PR benchmark tasks
- clones the upstream repo into `eval/workspaces/cache/`
- creates detached pre-fix git worktrees for `control` and `tg` runs
- writes redacted task prompts that hide the accepted PR
- injects tracegrep agent integration only into the `tg` condition
- generates launcher scripts for both `codex` and `claude`
- creates blind diff artifacts for `control` and `tg`
- runs an LLM judge against the human PR diff
- optionally publishes both branches to GitHub forks under `btucker`
- renders per-task markdown reports and an aggregate matrix under `eval/reports/`

## Usage

List tasks:

```bash
uv run eval/benchmark.py list
```

Inspect one task:

```bash
uv run eval/benchmark.py show storybook-hide-toolbar-docs
```

Discover recent candidate tasks with an agent-assisted shortlist:

```bash
uv run eval/benchmark.py discover --agent codex
uv run eval/benchmark.py discover --agent claude --model sonnet --pr-cutoff 2025-09-10
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

Judge one task after the agent runs finish:

```bash
uv run eval/benchmark.py judge storybook-hide-toolbar-docs --agent codex
uv run eval/benchmark.py judge storybook-hide-toolbar-docs --agent claude --judge-agent codex --judge-model gpt-5
```

Run the full task flow with one command:

```bash
uv run eval/benchmark.py run-task storybook-hide-toolbar-docs --agent codex
uv run eval/benchmark.py run-task storybook-hide-toolbar-docs --agent claude --judge-agent codex --judge-model gpt-5 -- --model sonnet
```

Publish the latest evaluated branches to the GitHub fork:

```bash
uv run eval/benchmark.py publish storybook-hide-toolbar-docs --agent codex
```

Render or refresh the markdown report:

```bash
uv run eval/benchmark.py report storybook-hide-toolbar-docs --agent codex
uv run eval/benchmark.py report-all --agent codex
```

Pass extra flags to the underlying CLI after `--`:

```bash
uv run eval/benchmark.py launch vite-import-meta-glob-base --agent codex --condition tg -- --model gpt-5
uv run eval/benchmark.py launch vite-import-meta-glob-base --agent claude --condition tg -- --model sonnet --permission-mode acceptEdits
```

Model selection notes:

- Arguments after `--` are passed only to the build runs.
- `--judge-model` applies only to the blind judge run.
- If `--judge-model` is omitted, the judge CLI uses its default model.

## Output layout

Preparing a task creates:

- `eval/workspaces/cache/<repo>/` shared upstream clone
- `eval/workspaces/runs/<task>/worktrees/<condition>/` detached repo snapshot
- `eval/workspaces/runs/<task>/prompts/<condition>.md` agent prompt
- `eval/workspaces/runs/<task>/launch_<agent>_<condition>.sh` launcher script
- `eval/workspaces/runs/<task>/hidden/ground_truth.json` accepted PR metadata for evaluation

Discovery creates:

- `eval/workspaces/discovery/<timestamp>-<agent>/raw_candidates.json`
- `eval/workspaces/discovery/<timestamp>-<agent>/selection_prompt.md`
- `eval/workspaces/discovery/<timestamp>-<agent>/selection.json`
- `eval/workspaces/discovery/<timestamp>-<agent>/shortlist.json`
- `eval/workspaces/discovery/<timestamp>-<agent>/report.md`

Judging a task creates:

- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/control.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/tg.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/ground_truth.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/blind_manifest.json`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judge_input.json`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judgment.json`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/publish.json`

Rendering reports writes repo-tracked markdown artifacts:

- `eval/reports/<task>/<agent>/<eval-id>.md`

Aggregate reporting writes:

- `eval/reports/matrix.md`
- `eval/reports/matrix.json`

For the `tg` worktree only, `prepare` also creates:

- `.codex/skills/tracegrep/` copied from this repo's `skills/tracegrep/`
- `.claude/settings.local.json` enabling the `tracegrep@tracegrep-dev` plugin marketplace entry
- `.eval-bin/tg` copied from the host `tg` binary so the workspace sandbox can execute it
- `.tracegrep-cache/` for cache writes inside the prepared worktree
- the generated Claude `tg` launcher checks `claude plugin list --json` and fails fast if `tracegrep@tracegrep-dev` is not installed
- the generated `tg` launchers run `tg --build-index` from inside the prepared worktree before handing control to the agent

## Notes

- The prompts tell agents not to browse the web or inspect PR history.
- The `control` prompt explicitly forbids `tg`.
- The `tg` prompt explicitly prefers `tg`/`tracegrep` for supported source files.
- The benchmark is about agent use of `tg`, so the `tg` condition includes the agent-specific skill/plugin wiring needed to expose it naturally.
- The generated launchers prepend `.eval-bin/` to `PATH` and set `TRACEGREP_CACHE_DIR` to `.tracegrep-cache/` so `tg` stays usable inside the workspace sandbox.
- The generated `tg` launchers also prewarm the repo index inside the worktree, so the first agent search is less dominated by cold graph construction.
- The `control` worktree does not get the Codex skill or Claude plugin config.
- For tasks where the original issue contained solution leakage, the manifest uses a redacted benchmark prompt instead.
- The judge stays blind to which side is `control` vs `tg`; that mapping is only revealed in the final markdown report.
- The default judge agent comes from `TRACEGREP_EVAL_JUDGE_AGENT` and falls back to `claude`.
- `publish` uses `gh` to detect or create forks under `btucker`, then pushes both branches with opaque benchmark branch names.
- Public publishing can leak benchmark solutions into future search. Treat published branches as post-hoc artifacts, not inputs to new runs.
- The markdown reports are meant to be committed back into this repo; the disposable run artifacts stay under `eval/workspaces/`.
- `run-task` is the one-command sequential workflow: prepare, launch control, launch tg, judge, publish, and render the report.
- `discover` uses GitHub search plus a structured `codex` or `claude` pass to shortlist issue/PR pairs for future benchmark additions.
- `discover` filters to MIT-licensed public repos, skips repos already present in `tasks.json`, and currently limits the pool to tracegrep-supported primary languages (`JavaScript`, `TypeScript`, `Python`, `Rust`).
- The default discovery recency gate is PR merged on or after six months before the run date. Override `--pr-cutoff YYYY-MM-DD` if you want a stricter or looser approximation of training-data freshness.
