# Eval Harness

This directory contains a small benchmark harness for comparing `codex` and `claude` CLI runs with and without agent access to `tg`.

The claim this harness is built to test is:

- does having the agent use `tg` make a difference in the code it produces?

## Methodology

Each benchmark task is a historical issue/PR pair captured in `tasks.json`. The harness uses the issue text as the task prompt and treats the accepted human PR as the reference implementation for evaluation.

For each task, the harness creates two detached worktrees from the same pre-fix commit:

- `control`: the agent works without `tg`
- `tg`: the agent gets the `tracegrep` integration and is expected to use it naturally

The goal is not to compare two arbitrary solutions. The goal is to hold as much constant as possible and isolate one variable: whether giving the agent access to `tg` changes the implementation it produces.

The task flow is:

1. Prepare both worktrees from the same parent commit.
2. Give both variants the same benchmark prompt, except that `control` forbids `tg` and `tg` prefers it.
3. Let the agent modify each worktree independently.
4. Snapshot the resulting repos and generate three parent-based diffs:
   - parent -> control
   - parent -> tg
   - parent -> accepted human PR
5. Blind the two benchmark implementations as `A` and `B`.
6. Ask a judge model to do a three-way comparison across `A`, `B`, and the accepted PR.
7. Reveal which of `A` or `B` was `control` vs `tg` only after judgment.

The judge is allowed to inspect repo context, but it does not inspect the live `control` or `tg` worktrees directly. Instead, it receives blinded artifacts under `judge_workspace/`:

- `A.diff`, `B.diff`, and `accepted_pr.diff`
- sanitized exports of the resulting repo trees for `A`, `B`, and the accepted PR

That design matters because the live `tg` worktree contains benchmark harness wiring such as `.codex/`, `.claude/`, `.eval-bin/`, and `.tracegrep-cache/`, which would otherwise leak the condition to the judge.

The build prompts also explicitly forbid networked or baseline-changing git commands such as `git fetch`, `git pull`, and `git checkout`, so agents cannot refresh the repo or consult remote history mid-run.

The outputs should be read as comparative judgments, not absolute truth. The score columns and winners come from an LLM judge, and the per-task markdown reports contain the qualitative rationale behind the matrix. The accepted PR is treated as a reference and can rank above both benchmark implementations in the three-way ranking.

In normal use, `run-task` is the main entrypoint. It runs the whole benchmark flow for one task: prepare, launch `control`, launch `tg`, judge, publish, and render the report. The lower-level commands are still useful for inspection, retries, and debugging, but they are not the primary workflow.

## What it does

- keeps a manifest of historical issue/PR benchmark tasks
- can discover new candidate issue/PR pairs from GitHub for future benchmark additions
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

### Recommended: run the whole flow

If you want to benchmark one task end to end, use `run-task`:

```bash
uv run eval/benchmark.py run-task storybook-hide-toolbar-docs --agent codex
uv run eval/benchmark.py run-task storybook-hide-toolbar-docs --agent claude --agent-model sonnet --judge-agent codex --judge-model gpt-5
```

`run-task` is the default high-level workflow. It prepares both worktrees, runs both conditions, judges the result, optionally publishes branches, and writes the markdown report.

### Inspect current state

Discover recent candidate tasks with an agent-assisted shortlist:

```bash
uv run eval/benchmark.py discover --agent codex
uv run eval/benchmark.py discover --agent claude --model sonnet --pr-cutoff 2025-09-10
```

List evaluation runs and their current state:

```bash
uv run eval/benchmark.py
uv run eval/benchmark.py runs
```

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

Judge one task after the agent runs finish:

```bash
uv run eval/benchmark.py judge storybook-hide-toolbar-docs --agent codex
uv run eval/benchmark.py judge storybook-hide-toolbar-docs --agent claude --judge-agent codex --judge-model gpt-5
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

`report-all` skips tasks whose latest run is missing or unjudged and prints those skipped runs before writing the aggregate matrix.

Pass extra flags to the underlying CLI after `--`:

```bash
uv run eval/benchmark.py launch vite-import-meta-glob-base --agent codex --condition tg -- --model gpt-5
uv run eval/benchmark.py launch vite-import-meta-glob-base --agent claude --condition tg -- --model sonnet --permission-mode acceptEdits
```

Model selection notes:

- Arguments after `--` are passed only to the build runs.
- `--agent-model` applies to the build runs in `run-task`.
- `--judge-model` applies only to the blind judge run.
- If `--judge-model` is omitted, the judge CLI uses its default model.
- `run-task` rejects using both `--agent-model` and a forwarded `--model`/`-m` after `--` at the same time.

## Output layout

Discovery creates:

- `eval/workspaces/discovery/<timestamp>-<agent>/raw_candidates.json`
- `eval/workspaces/discovery/<timestamp>-<agent>/selection_prompt.md`
- `eval/workspaces/discovery/<timestamp>-<agent>/selection.json`
- `eval/workspaces/discovery/<timestamp>-<agent>/shortlist.json`
- `eval/workspaces/discovery/<timestamp>-<agent>/report.md`

Preparing a task creates:

- `eval/workspaces/cache/<repo>/` shared upstream clone
- `eval/workspaces/runs/<task>/worktrees/<condition>/` detached repo snapshot
- `eval/workspaces/runs/<task>/prompts/<condition>.md` agent prompt
- `eval/workspaces/runs/<task>/launch_<agent>_<condition>.sh` launcher script
- `eval/workspaces/runs/<task>/hidden/ground_truth.json` accepted PR metadata for evaluation

Judging a task creates:

- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/control.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/tg.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/ground_truth.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/blind_manifest.json`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judge_input.json`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judge_workspace/A.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judge_workspace/B.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judge_workspace/accepted_pr.diff`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judge_workspace/A_repo/`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judge_workspace/B_repo/`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judge_workspace/accepted_pr_repo/`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/judgment.json`
- `eval/workspaces/runs/<task>/evaluations/<agent>/<eval-id>/publish.json`

The default `runs` view shows one row per `evaluations/<agent>/<eval-id>/` directory, including:

- the task and agent
- the `eval-id` run id
- per-variant state for `control` and `tg`
- an overall run status

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
- The prompts also forbid `git fetch`, `git pull`, `git checkout`, and similar commands that would consult remotes or change the benchmark baseline.
- The `control` prompt explicitly forbids `tg`.
- The `tg` prompt explicitly prefers `tg`/`tracegrep` for supported source files.
- The benchmark is about agent use of `tg`, so the `tg` condition includes the agent-specific skill/plugin wiring needed to expose it naturally.
- The generated launchers prepend `.eval-bin/` to `PATH` and set `TRACEGREP_CACHE_DIR` to `.tracegrep-cache/` so `tg` stays usable inside the workspace sandbox.
- The generated `tg` launchers also prewarm the repo index inside the worktree, so the first agent search is less dominated by cold graph construction.
- The `control` worktree does not get the Codex skill or Claude plugin config.
- For tasks where the original issue contained solution leakage, the manifest uses a redacted benchmark prompt instead.
- The judge stays blind to which side is `control` vs `tg`; that mapping is only revealed in the final markdown report.
- The judge compares three parent-based diffs: parent -> A, parent -> B, and parent -> accepted PR.
- The judge gets tool access, but only through blinded `judge_workspace/` artifacts and sanitized repo exports, not the live `control/` or `tg` worktrees.
- The default judge agent comes from `TRACEGREP_EVAL_JUDGE_AGENT` and falls back to `claude`.
- `publish` uses `gh` to detect or create forks under `btucker`, then pushes both branches with opaque benchmark branch names.
- Public publishing can leak benchmark solutions into future search. Treat published branches as post-hoc artifacts, not inputs to new runs.
- The markdown reports are meant to be committed back into this repo; the disposable run artifacts stay under `eval/workspaces/`.
- `run-task` is the one-command sequential workflow: prepare, launch control, launch tg, judge, publish, and render the report.
- `discover` uses GitHub search plus a structured `codex` or `claude` pass to shortlist issue/PR pairs for future benchmark additions.
- `discover` filters to MIT-licensed public repos, skips repos already present in `tasks.json`, and currently limits the pool to tracegrep-supported primary languages (`JavaScript`, `TypeScript`, `Python`, `Rust`).
- The default discovery recency gate is PRs merged on or after about six months before the run date. Override `--pr-cutoff YYYY-MM-DD` if you want a stricter or looser approximation of training-data freshness.
