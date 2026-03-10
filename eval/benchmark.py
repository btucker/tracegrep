#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# ///

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import textwrap
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import quote


SCRIPT_DIR = Path(__file__).resolve().parent
TASKS_PATH = SCRIPT_DIR / "tasks.json"
DEFAULT_ROOT = SCRIPT_DIR / "workspaces"
DEFAULT_FORK_OWNER = "btucker"
DEFAULT_JUDGE_AGENT = "claude"
SUPPORTED_AGENTS = ("codex", "claude")
SUPPORTED_CONDITIONS = ("control", "tg")
TRACEGREP_SKILL_SOURCE = SCRIPT_DIR.parent / "skills" / "tracegrep"
BENCHMARK_EXPORT_NAME = "tracegrep-eval"
BENCHMARK_EXPORT_EMAIL = "tracegrep-eval@example.com"

JUDGE_SCHEMA: dict[str, Any] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "better_matches_pr",
        "better_overall",
        "confidence",
        "scores",
        "A_vs_pr_differences",
        "B_vs_pr_differences",
        "A_vs_B_differences",
        "notable_strengths",
        "notable_risks",
        "summary",
    ],
    "properties": {
        "better_matches_pr": {"type": "string", "enum": ["A", "B", "tie"]},
        "better_overall": {"type": "string", "enum": ["A", "B", "tie"]},
        "confidence": {"type": "string", "enum": ["low", "medium", "high"]},
        "scores": {
            "type": "object",
            "additionalProperties": False,
            "required": ["A", "B"],
            "properties": {
                "A": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": [
                        "pr_alignment",
                        "reuse_alignment",
                        "duplication_risk",
                        "test_alignment",
                    ],
                    "properties": {
                        "pr_alignment": {"type": "integer", "minimum": 1, "maximum": 5},
                        "reuse_alignment": {"type": "integer", "minimum": 1, "maximum": 5},
                        "duplication_risk": {"type": "integer", "minimum": 1, "maximum": 5},
                        "test_alignment": {"type": "integer", "minimum": 1, "maximum": 5},
                    },
                },
                "B": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": [
                        "pr_alignment",
                        "reuse_alignment",
                        "duplication_risk",
                        "test_alignment",
                    ],
                    "properties": {
                        "pr_alignment": {"type": "integer", "minimum": 1, "maximum": 5},
                        "reuse_alignment": {"type": "integer", "minimum": 1, "maximum": 5},
                        "duplication_risk": {"type": "integer", "minimum": 1, "maximum": 5},
                        "test_alignment": {"type": "integer", "minimum": 1, "maximum": 5},
                    },
                },
            },
        },
        "A_vs_pr_differences": {
            "type": "array",
            "items": {"type": "string"},
            "maxItems": 10,
        },
        "B_vs_pr_differences": {
            "type": "array",
            "items": {"type": "string"},
            "maxItems": 10,
        },
        "A_vs_B_differences": {
            "type": "array",
            "items": {"type": "string"},
            "maxItems": 10,
        },
        "notable_strengths": {
            "type": "object",
            "additionalProperties": False,
            "required": ["A", "B"],
            "properties": {
                "A": {"type": "array", "items": {"type": "string"}, "maxItems": 8},
                "B": {"type": "array", "items": {"type": "string"}, "maxItems": 8},
            },
        },
        "notable_risks": {
            "type": "object",
            "additionalProperties": False,
            "required": ["A", "B"],
            "properties": {
                "A": {"type": "array", "items": {"type": "string"}, "maxItems": 8},
                "B": {"type": "array", "items": {"type": "string"}, "maxItems": 8},
            },
        },
        "summary": {"type": "string"},
    },
}


def load_tasks() -> dict[str, dict[str, Any]]:
    tasks = json.loads(TASKS_PATH.read_text())
    return {task["id"]: task for task in tasks}


def repo_slug(task: dict[str, Any]) -> str:
    return task["repo"]["name"].replace("/", "__")


def repo_basename(task: dict[str, Any]) -> str:
    return task["repo"]["name"].split("/", 1)[1]


def cache_repo_dir(root: Path, task: dict[str, Any]) -> Path:
    return root / "cache" / repo_slug(task)


def agent_runs_dir(root: Path, task_id: str, agent: str) -> Path:
    return root / "runs" / task_id / agent


def run_dir(root: Path, task_id: str, agent: str, run_id: str) -> Path:
    return agent_runs_dir(root, task_id, agent) / run_id


def run_manifest_path(root: Path, task_id: str, agent: str, run_id: str) -> Path:
    return run_dir(root, task_id, agent, run_id) / "manifest.json"


def prompt_path(root: Path, task_id: str, agent: str, run_id: str, condition: str) -> Path:
    return run_dir(root, task_id, agent, run_id) / "prompts" / f"{condition}.md"


def worktree_dir(root: Path, task_id: str, agent: str, run_id: str, condition: str) -> Path:
    return run_dir(root, task_id, agent, run_id) / "worktrees" / condition


def launch_script_path(root: Path, task_id: str, agent: str, run_id: str, condition: str) -> Path:
    return run_dir(root, task_id, agent, run_id) / f"launch_{agent}_{condition}.sh"


def hidden_ground_truth_path(root: Path, task_id: str, agent: str, run_id: str) -> Path:
    return run_dir(root, task_id, agent, run_id) / "hidden" / "ground_truth.json"


def reports_dir(root: Path) -> Path:
    return root.parent / "reports"


def run_report_path(root: Path, task_id: str, agent: str, run_id: str) -> Path:
    return reports_dir(root) / task_id / agent / f"{run_id}.md"


def local_tg_path(worktree: Path) -> Path:
    return worktree / ".eval-bin" / "tg"


def local_cache_root(worktree: Path) -> Path:
    return worktree / ".tracegrep-cache"


def load_run_manifest(run_path: Path) -> dict[str, Any]:
    return load_json(run_path / "manifest.json")


def write_run_manifest(run_path: Path, manifest: dict[str, Any]) -> None:
    write_json(run_path / "manifest.json", manifest)


def run(
    args: list[str],
    *,
    cwd: Path | None = None,
    capture_output: bool = False,
    check: bool = True,
    env: dict[str, str] | None = None,
    input_text: str | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=str(cwd) if cwd else None,
        check=check,
        text=True,
        capture_output=capture_output,
        env=env,
        input=input_text,
    )


def ensure_root(root: Path) -> None:
    (root / "cache").mkdir(parents=True, exist_ok=True)
    (root / "runs").mkdir(parents=True, exist_ok=True)
    reports_dir(root).mkdir(parents=True, exist_ok=True)


def ensure_repo_cache(root: Path, task: dict[str, Any]) -> Path:
    cache_dir = cache_repo_dir(root, task)
    repo_url = task["repo"]["url"]
    if not cache_dir.exists():
        run(["git", "clone", "--filter=blob:none", repo_url, str(cache_dir)])
    run(["git", "fetch", "--all", "--tags", "--prune"], cwd=cache_dir)
    return cache_dir


def remove_existing_worktree(cache_dir: Path, path: Path) -> None:
    if not path.exists():
        return
    try:
        run(["git", "worktree", "remove", "--force", str(path)], cwd=cache_dir)
    except subprocess.CalledProcessError:
        pass
    if path.exists():
        shutil.rmtree(path)
    run(["git", "worktree", "prune"], cwd=cache_dir)


def ensure_worktree(
    root: Path,
    task: dict[str, Any],
    evaluated_agent: str,
    run_id: str,
    condition: str,
    *,
    force: bool,
) -> Path:
    cache_dir = ensure_repo_cache(root, task)
    path = worktree_dir(root, task["id"], evaluated_agent, run_id, condition)
    commit = task["ground_truth"]["pre_fix_commit"]
    if path.exists():
        if not force:
            return path
        remove_existing_worktree(cache_dir, path)
    path.parent.mkdir(parents=True, exist_ok=True)
    run(["git", "worktree", "add", "--detach", str(path), commit], cwd=cache_dir)
    return path


def remove_tree(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    if path.exists():
        shutil.rmtree(path)


def write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")


def load_json(path: Path) -> Any:
    return json.loads(path.read_text())


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def host_tg_binary() -> Path:
    tg = shutil.which("tg")
    if tg is None:
        raise SystemExit("`tg` was not found on PATH")
    return Path(tg).resolve()


def require_command(name: str) -> str:
    path = shutil.which(name)
    if path is None:
        raise SystemExit(f"`{name}` was not found on PATH")
    return path


def write_claude_settings(path: Path) -> None:
    settings = {
        "$schema": "https://json.schemastore.org/claude-code-settings.json",
        "enabledPlugins": {
            "tracegrep@tracegrep-dev": True,
        },
        "extraKnownMarketplaces": {
            "tracegrep-dev": {
                "source": {
                    "source": "github",
                    "repo": "btucker/tracegrep",
                    "ref": "main",
                }
            }
        },
    }
    write_json(path, settings)


def configure_condition_environment(worktree: Path, condition: str) -> None:
    codex_skill_dir = worktree / ".codex" / "skills" / "tracegrep"
    claude_settings_path = worktree / ".claude" / "settings.local.json"
    tg_binary_path = local_tg_path(worktree)
    cache_root = local_cache_root(worktree)

    if condition == "tg":
        if not TRACEGREP_SKILL_SOURCE.exists():
            raise SystemExit(f"tracegrep skill source not found at {TRACEGREP_SKILL_SOURCE}")
        remove_tree(codex_skill_dir)
        codex_skill_dir.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(TRACEGREP_SKILL_SOURCE, codex_skill_dir)
        claude_settings_path.parent.mkdir(parents=True, exist_ok=True)
        write_claude_settings(claude_settings_path)
        tg_binary_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(host_tg_binary(), tg_binary_path)
        tg_binary_path.chmod(0o755)
        cache_root.mkdir(parents=True, exist_ok=True)
        return

    remove_tree(codex_skill_dir)
    if claude_settings_path.exists():
        claude_settings_path.unlink()
    remove_tree(tg_binary_path)
    remove_tree(cache_root)


def condition_search_guidance(condition: str) -> str:
    if condition == "tg":
        return (
            "Search guidance for this run: when searching supported source files "
            "(`.rs`, `.py`, `.js`, `.jsx`, `.ts`, `.tsx`, `.svelte`), prefer `tg`/`tracegrep` "
            "over raw `rg` or `grep` because it returns call-graph context. Only fall back to raw "
            "text search if `tg` cannot model the target."
        )
    return (
        "Search guidance for this run: use your normal repo exploration workflow, but do not use "
        "`tg` or `tracegrep` in this run."
    )


def build_prompt(task: dict[str, Any], condition: str) -> str:
    body = task["prompt"]["body"].strip()
    parts = [
        "# Benchmark Task",
        f"Task: {task['prompt']['title']}",
        body,
        textwrap.dedent(
            """\
            Constraints:
            - Work only from the checked-out repository state and this prompt.
            - Do not browse the web, open GitHub issues or PRs, or inspect repository history beyond the current checkout.
            - Keep the change consistent with the surrounding code and tests.
            - Run relevant tests or checks before finishing, and mention what you ran.
            """
        ).strip(),
        condition_search_guidance(condition),
    ]
    return "\n\n".join(parts) + "\n"


def build_run_readme(task: dict[str, Any], root: Path, evaluated_agent: str, run_id: str) -> str:
    base = run_dir(root, task["id"], evaluated_agent, run_id)
    lines = [
        f"# {task['id']} / {evaluated_agent} / {run_id}",
        "",
        f"- Repo: {task['repo']['name']}",
        f"- License: {task['repo']['license']}",
        f"- Language: {task['repo']['language']}",
        f"- Issue: {task['issue']['url']}",
        f"- Hidden PR ground truth: {task['ground_truth']['pr_url']}",
        f"- Run ID: {run_id}",
        "",
        "Launchers:",
    ]
    for condition in SUPPORTED_CONDITIONS:
        script = launch_script_path(root, task["id"], evaluated_agent, run_id, condition)
        lines.append(f"- {script.name}")
    lines.extend(
        [
            "",
            "Prompts:",
            f"- {prompt_path(root, task['id'], evaluated_agent, run_id, 'control')}",
            f"- {prompt_path(root, task['id'], evaluated_agent, run_id, 'tg')}",
            "",
            "Worktrees:",
            f"- {worktree_dir(root, task['id'], evaluated_agent, run_id, 'control')}",
            f"- {worktree_dir(root, task['id'], evaluated_agent, run_id, 'tg')}",
            "",
            "tg condition environment additions:",
            "- `.codex/skills/tracegrep/` copied from this repo",
            "- `.claude/settings.local.json` enabling `tracegrep@tracegrep-dev`",
            "- `.eval-bin/tg` copied from the host `tg` binary so the workspace sandbox can execute it",
            "- `.tracegrep-cache/` used via `TRACEGREP_CACHE_DIR` to keep cache writes inside the worktree",
            "",
            "Run flow:",
            "- `judge` creates blind comparison artifacts in this run directory",
            "- `publish` pushes both condition snapshots to the GitHub fork under `btucker`",
            "- `report` renders or refreshes a markdown report for the run",
        ]
    )
    return "\n".join(lines) + "\n"


def build_launcher_script(task: dict[str, Any], root: Path, agent: str, run_id: str, condition: str) -> str:
    base = run_dir(root, task["id"], agent, run_id)
    prompt_file = base / "prompts" / f"{condition}.md"
    worktree = base / "worktrees" / condition
    tg_path = local_tg_path(worktree)
    cache_root = local_cache_root(worktree)
    if agent == "codex":
        command = 'exec codex --full-auto "$@" "$(cat "$PROMPT_FILE")"'
    elif agent == "claude":
        command = 'exec claude "$@" "$(cat "$PROMPT_FILE")"'
    else:
        raise ValueError(f"unsupported agent: {agent}")
    preflight_lines: list[str] = []
    env_setup_lines: list[str] = []
    warmup_lines: list[str] = []
    if condition == "tg":
        env_setup_lines = [
            f'export PATH="{tg_path.parent}:$PATH"',
            f'export TRACEGREP_CACHE_DIR="{cache_root}"',
        ]
        warmup_lines = [
            'echo "Prebuilding tracegrep index in $WORKTREE..."',
            "tg --build-index",
        ]
    if agent == "claude" and condition == "tg":
        preflight_lines = [
            'PLUGIN_ID="tracegrep@tracegrep-dev"',
            "if ! claude plugin list --json | python3 -c '",
            "import json",
            "import sys",
            "",
            "plugin_id = sys.argv[1]",
            "plugins = json.load(sys.stdin)",
            'installed = any(plugin.get("id") == plugin_id for plugin in plugins)',
            "sys.exit(0 if installed else 1)",
            '\' "$PLUGIN_ID"; then',
            '  echo "Required Claude plugin not installed: $PLUGIN_ID" >&2',
            '  echo "Run this once from the worktree to install it:" >&2',
            '  echo "  claude plugin install $PLUGIN_ID" >&2',
            "  exit 1",
            "fi",
        ]

    lines = [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        "",
        'BASE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"',
        f'WORKTREE="{worktree}"',
        f'PROMPT_FILE="{prompt_file}"',
        "",
        'cd "$WORKTREE"',
    ]
    if env_setup_lines:
        lines.extend(env_setup_lines)
        lines.append("")
    if preflight_lines:
        lines.extend(preflight_lines)
        lines.append("")
    if warmup_lines:
        lines.extend(warmup_lines)
        lines.append("")
    lines.append(command)
    return "\n".join(lines) + "\n"


def prepare_run(root: Path, task: dict[str, Any], *, evaluated_agent: str, run_id: str, force: bool) -> None:
    base = run_dir(root, task["id"], evaluated_agent, run_id)
    (base / "prompts").mkdir(parents=True, exist_ok=True)
    (base / "hidden").mkdir(parents=True, exist_ok=True)

    for condition in SUPPORTED_CONDITIONS:
        worktree = ensure_worktree(root, task, evaluated_agent, run_id, condition, force=force)
        configure_condition_environment(worktree, condition)
        prompt = build_prompt(task, condition)
        prompt_path(root, task["id"], evaluated_agent, run_id, condition).write_text(prompt)

    hidden_payload = {
        "task_id": task["id"],
        "repo": task["repo"],
        "issue": task["issue"],
        "ground_truth": task["ground_truth"],
        "evaluation_focus": task["evaluation_focus"],
    }
    write_json(hidden_ground_truth_path(root, task["id"], evaluated_agent, run_id), hidden_payload)
    (base / "README.md").write_text(build_run_readme(task, root, evaluated_agent, run_id))

    for condition in SUPPORTED_CONDITIONS:
        script_path = launch_script_path(root, task["id"], evaluated_agent, run_id, condition)
        script_path.write_text(build_launcher_script(task, root, evaluated_agent, run_id, condition))
        os.chmod(script_path, 0o755)

    manifest = load_run_manifest(base)
    for condition in SUPPORTED_CONDITIONS:
        manifest["variants"][condition]["prepared"] = True
    write_run_manifest(base, manifest)


def cmd_list(tasks: dict[str, dict[str, Any]]) -> int:
    for task in tasks.values():
        print(
            f"{task['id']}: {task['repo']['name']} | "
            f"{task['prompt']['title']} | issue #{task['issue']['number']}"
        )
    return 0


def cmd_show(tasks: dict[str, dict[str, Any]], task_id: str) -> int:
    task = tasks[task_id]
    print(f"id: {task['id']}")
    print(f"repo: {task['repo']['name']} ({task['repo']['license']}, {task['repo']['language']})")
    print(f"issue: #{task['issue']['number']} {task['issue']['url']}")
    print(f"ground truth pr: #{task['ground_truth']['pr_number']} {task['ground_truth']['pr_url']}")
    print(f"pre-fix commit: {task['ground_truth']['pre_fix_commit']}")
    print("")
    print(task["prompt"]["title"])
    print(task["prompt"]["body"])
    print("")
    print("evaluation focus:")
    for item in task["evaluation_focus"]:
        print(f"- {item}")
    return 0


def cmd_list_runs(
    tasks: dict[str, dict[str, Any]],
    task_ids: list[str],
    root: Path,
    *,
    evaluated_agent: str | None,
) -> int:
    selected = task_ids or list(tasks.keys())
    for task_id in selected:
        agents = [evaluated_agent] if evaluated_agent else list(SUPPORTED_AGENTS)
        for agent in agents:
            if agent is None:
                continue
            base = agent_runs_dir(root, task_id, agent)
            if not base.exists():
                continue
            for run_path in sorted(path for path in base.iterdir() if path.is_dir()):
                manifest_path = run_path / "manifest.json"
                if not manifest_path.exists():
                    continue
                manifest = load_run_manifest(run_path)
                print(
                    f"{task_id}\t{agent}\t{run_path.name}\t{derive_run_status(run_path, manifest)}\t"
                    f"{manifest['created_at']}"
                )
    return 0


def cmd_launch(
    tasks: dict[str, dict[str, Any]],
    task_id: str,
    root: Path,
    *,
    agent: str,
    run_id: str,
    condition: str,
    extra_args: list[str],
) -> int:
    task = tasks[task_id]
    run_path = resolve_run_dir(root, task_id, agent, run_id)
    script = launch_script_path(root, task_id, agent, run_id, condition)
    if not script.exists():
        raise SystemExit(
            f"{script} does not exist. Create the run with `run-task` before launching it."
        )
    manifest = load_run_manifest(run_path)
    if manifest["task_id"] != task["id"] or manifest["evaluated_agent"] != agent:
        raise SystemExit(f"Run {run_id} does not match task {task_id} agent {agent}.")
    launch_args = list(extra_args) if extra_args else list(manifest.get("build_args", []))
    command = [str(script), *launch_args]
    print("launching:", " ".join(shlex.quote(part) for part in command))
    completed = subprocess.run(command)
    variant_state = manifest["variants"][condition]
    variant_state["launched"] = completed.returncode == 0
    variant_state["last_launch_at"] = datetime.now(timezone.utc).isoformat()
    variant_state["last_exit_code"] = completed.returncode
    variant_state["last_args"] = launch_args
    write_run_manifest(run_path, manifest)
    return completed.returncode


def default_judge_agent() -> str:
    value = os.environ.get("TRACEGREP_EVAL_JUDGE_AGENT", DEFAULT_JUDGE_AGENT)
    if value not in SUPPORTED_AGENTS:
        raise SystemExit(
            "TRACEGREP_EVAL_JUDGE_AGENT must be one of: " + ", ".join(SUPPORTED_AGENTS)
        )
    return value


def forwarded_build_args(extra_args: list[str], agent_model: str | None) -> list[str]:
    if agent_model is None:
        return list(extra_args)
    model_flags = {"--model", "-m"}
    if any(arg in model_flags for arg in extra_args):
        raise SystemExit(
            "Do not pass both `--agent-model` and a forwarded `--model`/`-m` after `--`."
        )
    return ["--model", agent_model, *extra_args]


def new_run_id() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def new_run_manifest(
    *,
    task: dict[str, Any],
    evaluated_agent: str,
    run_id: str,
    agent_model: str | None,
    build_args: list[str],
) -> dict[str, Any]:
    return {
        "run_id": run_id,
        "task_id": task["id"],
        "evaluated_agent": evaluated_agent,
        "agent_model": agent_model,
        "build_args": list(build_args),
        "created_at": datetime.now(timezone.utc).isoformat(),
        "variants": {
            condition: {
                "prepared": False,
                "launched": False,
                "last_launch_at": None,
                "last_exit_code": None,
                "last_args": [],
            }
            for condition in SUPPORTED_CONDITIONS
        },
        "judge": {
            "completed": False,
            "judge_agent": None,
            "judge_model": None,
            "completed_at": None,
        },
        "publish": {
            "published": False,
            "published_at": None,
        },
    }


def create_run(
    root: Path,
    task: dict[str, Any],
    *,
    evaluated_agent: str,
    run_id: str,
    agent_model: str | None,
    build_args: list[str],
    force: bool,
) -> Path:
    ensure_root(root)
    path = run_dir(root, task["id"], evaluated_agent, run_id)
    if path.exists():
        if not force:
            raise SystemExit(f"Run {run_id} already exists at {path}")
        shutil.rmtree(path)
    path.mkdir(parents=True, exist_ok=False)
    write_run_manifest(
        path,
        new_run_manifest(
            task=task,
            evaluated_agent=evaluated_agent,
            run_id=run_id,
            agent_model=agent_model,
            build_args=build_args,
        ),
    )
    return path


def latest_run_dir(root: Path, task_id: str, agent: str, *, require_judgment: bool = False) -> Path:
    base = agent_runs_dir(root, task_id, agent)
    if not base.exists():
        raise SystemExit(
            f"No runs found for {task_id} agent {agent}. Run `run-task` first."
        )
    entries = sorted(path for path in base.iterdir() if path.is_dir())
    if require_judgment:
        entries = [path for path in entries if (path / "judgment.json").exists()]
    if not entries:
        if require_judgment:
            raise SystemExit(f"No judged runs found for {task_id} agent {agent}.")
        raise SystemExit(f"No runs found for {task_id} agent {agent}.")
    return entries[-1]


def resolve_run_dir(root: Path, task_id: str, agent: str, run_id: str) -> Path:
    path = run_dir(root, task_id, agent, run_id)
    if not path.exists():
        raise SystemExit(f"Run {run_id} does not exist for {task_id} agent {agent}.")
    return path


def derive_run_status(run_path: Path, manifest: dict[str, Any] | None = None) -> str:
    if manifest is None:
        manifest = load_run_manifest(run_path)
    publish_path = run_path / "publish.json"
    if publish_path.exists():
        publish_meta = normalize_publish_metadata(load_json(publish_path))
        if publish_meta and publish_meta.get("published"):
            return "published"
    if (run_path / "judgment.json").exists():
        return "judged"
    variants = manifest.get("variants", {})
    if variants and all(item.get("launched") for item in variants.values()):
        return "launched"
    if variants and all(item.get("prepared") for item in variants.values()):
        return "prepared"
    return "created"


def stable_token(*parts: str, length: int = 12) -> str:
    digest = hashlib.sha256("::".join(parts).encode("utf-8")).hexdigest()
    return digest[:length]


def build_blind_manifest(
    *,
    task_id: str,
    evaluated_agent: str,
    run_id: str,
    snapshot_commits: dict[str, str],
) -> dict[str, Any]:
    flip = int(stable_token(task_id, evaluated_agent, run_id, length=2), 16) % 2 == 1
    label_to_condition = {"A": "control", "B": "tg"}
    if flip:
        label_to_condition = {"A": "tg", "B": "control"}
    condition_to_label = {condition: label for label, condition in label_to_condition.items()}
    return {
        "task_id": task_id,
        "evaluated_agent": evaluated_agent,
        "run_id": run_id,
        "label_to_condition": label_to_condition,
        "condition_to_label": condition_to_label,
        "snapshot_commits": snapshot_commits,
    }


def branch_names(task_id: str, evaluated_agent: str, run_id: str) -> dict[str, str]:
    return {
        "control": f"runs/{task_id}/{evaluated_agent}/{run_id}/control",
        "tg": f"runs/{task_id}/{evaluated_agent}/{run_id}/tg",
    }


def benchmark_git_env(index_path: str) -> dict[str, str]:
    env = os.environ.copy()
    env["GIT_INDEX_FILE"] = index_path
    env.setdefault("GIT_AUTHOR_NAME", BENCHMARK_EXPORT_NAME)
    env.setdefault("GIT_AUTHOR_EMAIL", BENCHMARK_EXPORT_EMAIL)
    env.setdefault("GIT_COMMITTER_NAME", BENCHMARK_EXPORT_NAME)
    env.setdefault("GIT_COMMITTER_EMAIL", BENCHMARK_EXPORT_EMAIL)
    return env


def snapshot_worktree_commit(worktree: Path, base_commit: str, message: str) -> str:
    fd, index_path = tempfile.mkstemp(prefix="tracegrep-eval-index-")
    os.close(fd)
    try:
        env = benchmark_git_env(index_path)
        run(["git", "read-tree", base_commit], cwd=worktree, env=env)
        run(["git", "add", "-A", "."], cwd=worktree, env=env)
        tree = run(["git", "write-tree"], cwd=worktree, env=env, capture_output=True).stdout.strip()
        commit = run(
            ["git", "commit-tree", tree, "-p", base_commit, "-m", message],
            cwd=worktree,
            env=env,
            capture_output=True,
        ).stdout.strip()
        return commit
    finally:
        Path(index_path).unlink(missing_ok=True)


def git_diff(repo_dir: Path, base_rev: str, target_rev: str) -> str:
    return run(
        ["git", "diff", "--binary", "--no-ext-diff", base_rev, target_rev],
        cwd=repo_dir,
        capture_output=True,
    ).stdout


def parse_numstat(text: str) -> dict[str, dict[str, int | str]]:
    stats: dict[str, dict[str, int | str]] = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        added, deleted, path = line.split("\t", 2)
        stats[path] = {
            "added": -1 if added == "-" else int(added),
            "deleted": -1 if deleted == "-" else int(deleted),
        }
    return stats


def parse_name_status(text: str) -> dict[str, dict[str, str]]:
    statuses: dict[str, dict[str, str]] = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        status = parts[0]
        path = parts[-1]
        statuses[path] = {"status": status}
        if status.startswith("R") or status.startswith("C"):
            statuses[path]["previous_path"] = parts[1]
    return statuses


def diff_file_summary(repo_dir: Path, base_rev: str, target_rev: str) -> dict[str, Any]:
    numstat = parse_numstat(
        run(["git", "diff", "--numstat", base_rev, target_rev], cwd=repo_dir, capture_output=True).stdout
    )
    statuses = parse_name_status(
        run(
            ["git", "diff", "--name-status", base_rev, target_rev],
            cwd=repo_dir,
            capture_output=True,
        ).stdout
    )
    files = []
    for path in sorted(set(numstat) | set(statuses)):
        item: dict[str, Any] = {"path": path}
        item.update(statuses.get(path, {"status": "M"}))
        item.update(numstat.get(path, {"added": 0, "deleted": 0}))
        files.append(item)
    return {
        "base_rev": base_rev,
        "target_rev": target_rev,
        "file_count": len(files),
        "files": files,
    }


def write_diff_artifacts(
    *,
    task: dict[str, Any],
    root: Path,
    evaluated_agent: str,
    run_id: str,
    artifact_dir: Path,
) -> tuple[dict[str, str], dict[str, Any]]:
    pre_fix = task["ground_truth"]["pre_fix_commit"]
    cache_dir = ensure_repo_cache(root, task)
    snapshot_commits: dict[str, str] = {}
    summaries: dict[str, Any] = {}

    for condition in SUPPORTED_CONDITIONS:
        worktree = worktree_dir(root, task["id"], evaluated_agent, run_id, condition)
        if not worktree.exists():
            raise SystemExit(
                f"Worktree {worktree} does not exist. Create the run with `run-task` first."
            )
        snapshot = snapshot_worktree_commit(
            worktree,
            pre_fix,
            f"Benchmark snapshot for {task['id']} {evaluated_agent} {condition} {run_id}",
        )
        snapshot_commits[condition] = snapshot
        diff = git_diff(worktree, pre_fix, snapshot)
        summary = diff_file_summary(worktree, pre_fix, snapshot)
        write_text(artifact_dir / f"{condition}.diff", diff)
        write_json(artifact_dir / f"{condition}_files.json", summary)
        summaries[condition] = summary

    ground_truth_commit = task["ground_truth"]["merge_commit"]
    ground_truth_diff = git_diff(cache_dir, pre_fix, ground_truth_commit)
    ground_truth_summary = diff_file_summary(cache_dir, pre_fix, ground_truth_commit)
    write_text(artifact_dir / "ground_truth.diff", ground_truth_diff)
    write_json(artifact_dir / "ground_truth_files.json", ground_truth_summary)
    summaries["ground_truth"] = ground_truth_summary
    return snapshot_commits, summaries


def build_judge_input(
    task: dict[str, Any],
    blind_manifest: dict[str, Any],
    eval_dir: Path,
) -> dict[str, Any]:
    label_to_condition = blind_manifest["label_to_condition"]
    implementations = {}
    for label in ("A", "B"):
        condition = label_to_condition[label]
        implementations[label] = {
            "diff_path": f"{label}.diff",
            "diff": (eval_dir / f"{condition}.diff").read_text(),
            "files": load_json(eval_dir / f"{condition}_files.json"),
        }
    return {
        "task_id": task["id"],
        "repo": task["repo"],
        "issue": task["issue"],
        "prompt": task["prompt"],
        "evaluation_focus": task["evaluation_focus"],
        "ground_truth": {
            **task["ground_truth"],
            "diff_path": "ground_truth.diff",
            "diff": (eval_dir / "ground_truth.diff").read_text(),
            "files": load_json(eval_dir / "ground_truth_files.json"),
        },
        "implementations": implementations,
    }


def build_judge_prompt(judge_input: dict[str, Any]) -> str:
    return textwrap.dedent(
        f"""\
        You are evaluating two candidate implementations for a historical benchmark task.

        The judge must stay blind to which implementation came from the control condition and which came from the tracegrep condition.

        Your job:
        - Compare Implementation A and Implementation B against the accepted human PR diff.
        - Decide which implementation better matches the accepted PR.
        - Decide which implementation appears better on its own merits.
        - Focus on architectural fit, reuse of existing patterns, duplication risk, and test alignment.
        - Explain how each implementation differs from the accepted PR.
        - Output only JSON matching the provided schema.

        Task metadata:
        - Task ID: {judge_input['task_id']}
        - Repo: {judge_input['repo']['name']}
        - Issue: #{judge_input['issue']['number']} {judge_input['issue']['title']}
        - Prompt title: {judge_input['prompt']['title']}

        Benchmark prompt:
        {judge_input['prompt']['body']}

        Evaluation focus:
        {json.dumps(judge_input['evaluation_focus'], indent=2)}

        Changed-file summary for Implementation A:
        {json.dumps(judge_input['implementations']['A']['files'], indent=2)}

        Changed-file summary for Implementation B:
        {json.dumps(judge_input['implementations']['B']['files'], indent=2)}

        Changed-file summary for the accepted PR:
        {json.dumps(judge_input['ground_truth']['files'], indent=2)}

        Implementation A diff:
        ```diff
        {judge_input['implementations']['A']['diff']}
        ```

        Implementation B diff:
        ```diff
        {judge_input['implementations']['B']['diff']}
        ```

        Accepted human PR diff:
        ```diff
        {judge_input['ground_truth']['diff']}
        ```
        """
    ).strip() + "\n"


def extract_json_object(text: str) -> Any:
    decoder = json.JSONDecoder()
    for index, char in enumerate(text):
        if char not in "{[":
            continue
        try:
            value, _ = decoder.raw_decode(text[index:])
            return value
        except json.JSONDecodeError:
            continue
    raise ValueError("Could not find a JSON object in judge output.")


def unwrap_possible_json_payload(payload: Any) -> Any:
    if isinstance(payload, dict) and {"better_matches_pr", "better_overall", "scores"} <= set(payload):
        return payload
    if isinstance(payload, dict):
        for key in ("result", "content", "output", "message", "final", "final_message"):
            if key not in payload:
                continue
            value = payload[key]
            if isinstance(value, dict):
                unwrapped = unwrap_possible_json_payload(value)
                if isinstance(unwrapped, dict) and {"better_matches_pr", "better_overall", "scores"} <= set(unwrapped):
                    return unwrapped
            if isinstance(value, str):
                try:
                    return unwrap_possible_json_payload(json.loads(value))
                except json.JSONDecodeError:
                    continue
        if "messages" in payload and isinstance(payload["messages"], list):
            for item in payload["messages"]:
                unwrapped = unwrap_possible_json_payload(item)
                if isinstance(unwrapped, dict) and {"better_matches_pr", "better_overall", "scores"} <= set(unwrapped):
                    return unwrapped
    if isinstance(payload, list):
        for item in payload:
            unwrapped = unwrap_possible_json_payload(item)
            if isinstance(unwrapped, dict) and {"better_matches_pr", "better_overall", "scores"} <= set(unwrapped):
                return unwrapped
    return payload


def parse_judge_output(text: str) -> dict[str, Any]:
    stripped = text.strip()
    if not stripped:
        raise ValueError("Judge output was empty.")
    try:
        payload = json.loads(stripped)
    except json.JSONDecodeError:
        payload = extract_json_object(stripped)
    payload = unwrap_possible_json_payload(payload)
    if not isinstance(payload, dict):
        raise ValueError("Judge output was not a JSON object.")
    return payload


def validate_judgment(payload: dict[str, Any]) -> None:
    required_top = {
        "better_matches_pr",
        "better_overall",
        "confidence",
        "scores",
        "A_vs_pr_differences",
        "B_vs_pr_differences",
        "A_vs_B_differences",
        "notable_strengths",
        "notable_risks",
        "summary",
    }
    missing = required_top - set(payload)
    if missing:
        raise ValueError(f"Judgment missing required fields: {sorted(missing)}")
    if payload["better_matches_pr"] not in {"A", "B", "tie"}:
        raise ValueError("better_matches_pr must be A, B, or tie")
    if payload["better_overall"] not in {"A", "B", "tie"}:
        raise ValueError("better_overall must be A, B, or tie")
    if payload["confidence"] not in {"low", "medium", "high"}:
        raise ValueError("confidence must be low, medium, or high")
    for label in ("A", "B"):
        if label not in payload["scores"]:
            raise ValueError(f"scores missing {label}")
        for key in ("pr_alignment", "reuse_alignment", "duplication_risk", "test_alignment"):
            value = payload["scores"][label].get(key)
            if not isinstance(value, int) or not (1 <= value <= 5):
                raise ValueError(f"{label}.{key} must be an integer between 1 and 5")
    for key in ("A_vs_pr_differences", "B_vs_pr_differences", "A_vs_B_differences"):
        if not isinstance(payload[key], list) or not all(isinstance(item, str) for item in payload[key]):
            raise ValueError(f"{key} must be a list of strings")
    for key in ("notable_strengths", "notable_risks"):
        if not isinstance(payload[key], dict):
            raise ValueError(f"{key} must be an object")
        for label in ("A", "B"):
            value = payload[key].get(label)
            if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
                raise ValueError(f"{key}.{label} must be a list of strings")
    if not isinstance(payload["summary"], str):
        raise ValueError("summary must be a string")


def run_judge_claude(prompt: str, *, cwd: Path, judge_model: str | None) -> dict[str, Any]:
    require_command("claude")
    command = [
        "claude",
        "-p",
        "--output-format",
        "json",
        "--json-schema",
        json.dumps(JUDGE_SCHEMA),
        "--permission-mode",
        "default",
        "--tools",
        "",
    ]
    if judge_model:
        command.extend(["--model", judge_model])
    completed = run(
        command,
        cwd=cwd,
        capture_output=True,
        input_text=prompt,
    )
    payload = parse_judge_output(completed.stdout)
    validate_judgment(payload)
    return payload


def run_judge_codex(prompt: str, *, cwd: Path, judge_model: str | None) -> dict[str, Any]:
    require_command("codex")
    with tempfile.TemporaryDirectory(prefix="tracegrep-eval-schema-") as tmpdir:
        schema_path = Path(tmpdir) / "judge_schema.json"
        output_path = Path(tmpdir) / "judgment_output.json"
        write_json(schema_path, JUDGE_SCHEMA)
        command = [
            "codex",
            "exec",
            "--skip-git-repo-check",
            "--ephemeral",
            "--color",
            "never",
            "-s",
            "read-only",
            "-C",
            str(cwd),
            "--output-schema",
            str(schema_path),
            "-o",
            str(output_path),
        ]
        if judge_model:
            command.extend(["--model", judge_model])
        command.append("-")
        completed = run(
            command,
            cwd=cwd,
            capture_output=True,
            input_text=prompt,
        )
        if output_path.exists():
            raw = output_path.read_text()
        else:
            raw = completed.stdout
    payload = parse_judge_output(raw)
    validate_judgment(payload)
    return payload


def run_judge_agent(
    judge_agent: str,
    prompt: str,
    *,
    cwd: Path,
    judge_model: str | None,
) -> dict[str, Any]:
    if judge_agent == "claude":
        return run_judge_claude(prompt, cwd=cwd, judge_model=judge_model)
    if judge_agent == "codex":
        return run_judge_codex(prompt, cwd=cwd, judge_model=judge_model)
    raise SystemExit(f"Unsupported judge agent: {judge_agent}")


def normalize_publish_metadata(payload: Any) -> dict[str, Any] | None:
    if not isinstance(payload, dict):
        return None
    return payload


def reveal_winner(label: str, blind_manifest: dict[str, Any]) -> str:
    if label == "tie":
        return "tie"
    return blind_manifest["label_to_condition"][label]


def condition_scores(judgment: dict[str, Any], blind_manifest: dict[str, Any]) -> dict[str, dict[str, int]]:
    label_for = blind_manifest["condition_to_label"]
    return {
        condition: judgment["scores"][label_for[condition]]
        for condition in SUPPORTED_CONDITIONS
    }


def condition_list(judgment: dict[str, Any], blind_manifest: dict[str, Any], key: str) -> dict[str, list[str]]:
    label_for = blind_manifest["condition_to_label"]
    return {
        condition: judgment[f"{label_for[condition]}{key}"]
        for condition in SUPPORTED_CONDITIONS
    }


def condition_nested_list(
    judgment: dict[str, Any],
    blind_manifest: dict[str, Any],
    section: str,
) -> dict[str, list[str]]:
    label_for = blind_manifest["condition_to_label"]
    return {
        condition: judgment[section][label_for[condition]]
        for condition in SUPPORTED_CONDITIONS
    }


def markdown_link(label: str, url: str | None) -> str:
    if not url:
        return "n/a"
    return f"[{label}]({url})"


def format_bullets(items: list[str]) -> str:
    if not items:
        return "- None noted"
    return "\n".join(f"- {item}" for item in items)


def build_report_markdown(
    *,
    task: dict[str, Any],
    evaluated_agent: str,
    judge_agent: str,
    run_id: str,
    judgment: dict[str, Any],
    blind_manifest: dict[str, Any],
    publish_meta: dict[str, Any] | None,
    report_path: Path | None = None,
) -> str:
    scores = condition_scores(judgment, blind_manifest)
    differences_vs_pr = condition_list(judgment, blind_manifest, "_vs_pr_differences")
    strengths = condition_nested_list(judgment, blind_manifest, "notable_strengths")
    risks = condition_nested_list(judgment, blind_manifest, "notable_risks")
    overall_winner = reveal_winner(judgment["better_overall"], blind_manifest)
    pr_winner = reveal_winner(judgment["better_matches_pr"], blind_manifest)
    control_vs_tg = judgment["A_vs_B_differences"]
    link_section = [
        "Publishing has not been run yet for this run.",
        "",
        "Public branch publishing can contaminate future benchmarks, so publish only after the run is complete.",
    ]
    control_branch_link = "n/a"
    tg_branch_link = "n/a"
    if publish_meta and publish_meta.get("published") and "branches" in publish_meta:
        branches = publish_meta["branches"]
        compares = publish_meta["compare_urls"]
        control_branch_link = markdown_link("control branch", branches["control"]["url"])
        tg_branch_link = markdown_link("tg branch", branches["tg"]["url"])
        link_section = [
            f"- Fork: {markdown_link(publish_meta['fork']['name_with_owner'], publish_meta['fork']['url'])}",
            f"- Control: {control_branch_link}",
            f"- TG: {tg_branch_link}",
            f"- Control vs pre-fix: {markdown_link('compare', compares.get('control_vs_pre_fix'))}",
            f"- TG vs pre-fix: {markdown_link('compare', compares.get('tg_vs_pre_fix'))}",
            f"- Control vs TG: {markdown_link('compare', compares.get('control_vs_tg'))}",
            "",
            "Public branch publishing can contaminate future benchmarks, so these links should be generated only after the run is complete.",
        ]

    report_rel = str(report_path) if report_path else "n/a"
    mapping_rows = "\n".join(
        [
            "| Label | Condition |",
            "| --- | --- |",
            f"| A | {blind_manifest['label_to_condition']['A']} |",
            f"| B | {blind_manifest['label_to_condition']['B']} |",
        ]
    )
    score_rows = "\n".join(
        [
            "| Condition | PR Alignment | Reuse Alignment | Duplication Risk | Test Alignment |",
            "| --- | ---: | ---: | ---: | ---: |",
            f"| control | {scores['control']['pr_alignment']} | {scores['control']['reuse_alignment']} | {scores['control']['duplication_risk']} | {scores['control']['test_alignment']} |",
            f"| tg | {scores['tg']['pr_alignment']} | {scores['tg']['reuse_alignment']} | {scores['tg']['duplication_risk']} | {scores['tg']['test_alignment']} |",
        ]
    )
    return textwrap.dedent(
        f"""\
        # Benchmark Report: {task['id']}

        ## Task Metadata
        - Repo: {task['repo']['name']}
        - Issue: [#{task['issue']['number']}]({task['issue']['url']}) {task['issue']['title']}
        - Human PR: [#{task['ground_truth']['pr_number']}]({task['ground_truth']['pr_url']})
        - Evaluated agent: `{evaluated_agent}`
        - Judge agent: `{judge_agent}`
        - Run ID: `{run_id}`
        - Report path: `{report_rel}`

        ## Blind Verdict Summary
        - Better matches the accepted PR: `{pr_winner}`
        - Better overall: `{overall_winner}`
        - Judge confidence: `{judgment['confidence']}`

        ## Blind Mapping
        {mapping_rows}

        ## Score Table
        {score_rows}

        ## Control vs Human PR
        ### Key differences
        {format_bullets(differences_vs_pr['control'])}

        ### Strengths
        {format_bullets(strengths['control'])}

        ### Risks
        {format_bullets(risks['control'])}

        ## TG vs Human PR
        ### Key differences
        {format_bullets(differences_vs_pr['tg'])}

        ### Strengths
        {format_bullets(strengths['tg'])}

        ### Risks
        {format_bullets(risks['tg'])}

        ## Control vs TG
        {format_bullets(control_vs_tg)}

        ## Published GitHub Links
        {'\n'.join(link_section)}

        ## Final Summary
        {judgment['summary']}
        """
    ).strip() + "\n"


def build_matrix_entry(
    *,
    task: dict[str, Any],
    evaluated_agent: str,
    judge_agent: str,
    run_id: str,
    judgment: dict[str, Any],
    blind_manifest: dict[str, Any],
    publish_meta: dict[str, Any] | None,
    report_path: Path,
    root: Path,
) -> dict[str, Any]:
    scores = condition_scores(judgment, blind_manifest)
    branches = publish_meta["branches"] if publish_meta else {}
    try:
        report_ref = str(report_path.relative_to(root.parent))
    except ValueError:
        report_ref = str(report_path)
    return {
        "task_id": task["id"],
        "evaluated_agent": evaluated_agent,
        "judge_agent": judge_agent,
        "run_id": run_id,
        "better_matches_pr": reveal_winner(judgment["better_matches_pr"], blind_manifest),
        "better_overall": reveal_winner(judgment["better_overall"], blind_manifest),
        "confidence": judgment["confidence"],
        "control_pr_alignment": scores["control"]["pr_alignment"],
        "tg_pr_alignment": scores["tg"]["pr_alignment"],
        "control_reuse_alignment": scores["control"]["reuse_alignment"],
        "tg_reuse_alignment": scores["tg"]["reuse_alignment"],
        "control_duplication_risk": scores["control"]["duplication_risk"],
        "tg_duplication_risk": scores["tg"]["duplication_risk"],
        "control_branch_url": branches.get("control", {}).get("url"),
        "tg_branch_url": branches.get("tg", {}).get("url"),
        "report_path": report_ref,
    }


def build_matrix_markdown(entries: list[dict[str, Any]]) -> str:
    lines = [
        "# Benchmark Matrix",
        "",
        "| Task | Agent | Judge | Better vs PR | Better overall | PR align control | PR align tg | Reuse control | Reuse tg | Duplication risk control | Duplication risk tg | Confidence | Control branch | TG branch | Report |",
        "| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |",
    ]
    for entry in entries:
        lines.append(
            "| "
            + " | ".join(
                [
                    entry["task_id"],
                    entry["evaluated_agent"],
                    entry["judge_agent"],
                    entry["better_matches_pr"],
                    entry["better_overall"],
                    str(entry["control_pr_alignment"]),
                    str(entry["tg_pr_alignment"]),
                    str(entry["control_reuse_alignment"]),
                    str(entry["tg_reuse_alignment"]),
                    str(entry["control_duplication_risk"]),
                    str(entry["tg_duplication_risk"]),
                    entry["confidence"],
                    markdown_link("control", entry["control_branch_url"]),
                    markdown_link("tg", entry["tg_branch_url"]),
                    entry["report_path"],
                ]
            )
            + " |"
        )
    return "\n".join(lines) + "\n"


def render_report_for_run(
    *,
    task: dict[str, Any],
    evaluated_agent: str,
    judge_agent: str,
    root: Path,
    run_path: Path,
) -> Path:
    judgment = load_json(run_path / "judgment.json")
    blind_manifest = load_json(run_path / "blind_manifest.json")
    publish_meta = None
    publish_path = run_path / "publish.json"
    if publish_path.exists():
        publish_meta = normalize_publish_metadata(load_json(publish_path))
    report_path = run_report_path(root, task["id"], evaluated_agent, run_path.name)
    report = build_report_markdown(
        task=task,
        evaluated_agent=evaluated_agent,
        judge_agent=judge_agent,
        run_id=run_path.name,
        judgment=judgment,
        blind_manifest=blind_manifest,
        publish_meta=publish_meta,
        report_path=report_path,
    )
    write_text(report_path, report)
    return report_path


def cmd_judge(
    tasks: dict[str, dict[str, Any]],
    task_id: str,
    root: Path,
    *,
    evaluated_agent: str,
    run_id: str,
    judge_agent: str,
    judge_model: str | None,
) -> int:
    task = tasks[task_id]
    run_path = resolve_run_dir(root, task_id, evaluated_agent, run_id)
    judgment_path = run_path / "judgment.json"
    if judgment_path.exists():
        raise SystemExit(f"Run {run_id} already has a judgment at {judgment_path}")
    snapshot_commits, _ = write_diff_artifacts(
        task=task,
        root=root,
        evaluated_agent=evaluated_agent,
        run_id=run_id,
        artifact_dir=run_path,
    )
    blind_manifest = build_blind_manifest(
        task_id=task_id,
        evaluated_agent=evaluated_agent,
        run_id=run_id,
        snapshot_commits=snapshot_commits,
    )
    write_json(run_path / "blind_manifest.json", blind_manifest)
    judge_input = build_judge_input(task, blind_manifest, run_path)
    write_json(run_path / "judge_input.json", judge_input)
    prompt = build_judge_prompt(judge_input)
    write_text(run_path / "judge_prompt.md", prompt)
    judgment = run_judge_agent(judge_agent, prompt, cwd=run_path, judge_model=judge_model)
    judgment["judge_agent"] = judge_agent
    if judge_model is not None:
        judgment["judge_model"] = judge_model
    write_json(judgment_path, judgment)
    publish_path = run_path / "publish.json"
    if not publish_path.exists():
        write_json(
            publish_path,
            {
                "published": False,
                "warning": "Public branch publishing can contaminate future benchmarks. Publish only after the run is complete.",
            },
        )
    manifest = load_run_manifest(run_path)
    manifest["snapshot_commits"] = snapshot_commits
    manifest["judge"] = {
        "completed": True,
        "judge_agent": judge_agent,
        "judge_model": judge_model,
        "completed_at": datetime.now(timezone.utc).isoformat(),
    }
    write_run_manifest(run_path, manifest)
    report_path = render_report_for_run(
        task=task,
        evaluated_agent=evaluated_agent,
        judge_agent=judge_agent,
        root=root,
        run_path=run_path,
    )
    print(f"judged {task_id} run {run_id} at {run_path}")
    print(f"report: {report_path}")
    return 0


def ensure_fork_repo(task: dict[str, Any], *, owner: str = DEFAULT_FORK_OWNER) -> dict[str, str]:
    require_command("gh")
    repo_name = repo_basename(task)
    fork_name = f"{owner}/{repo_name}"
    view = run(
        ["gh", "repo", "view", fork_name, "--json", "nameWithOwner,url"],
        capture_output=True,
        check=False,
    )
    if view.returncode != 0:
        create = run(
            ["gh", "repo", "fork", task["repo"]["name"], "--clone=false", "--remote=false"],
            capture_output=True,
            check=False,
        )
        if create.returncode != 0:
            raise SystemExit(
                f"Failed to create fork {fork_name} via gh:\n{create.stderr.strip() or create.stdout.strip()}"
            )
        view = run(
            ["gh", "repo", "view", fork_name, "--json", "nameWithOwner,url"],
            capture_output=True,
        )
    data = json.loads(view.stdout)
    return {
        "name_with_owner": data["nameWithOwner"],
        "url": data["url"],
        "git_url": data["url"].rstrip("/") + ".git",
    }


def ensure_remote(cache_dir: Path, remote_name: str, remote_url: str) -> None:
    remote = run(["git", "remote", "get-url", remote_name], cwd=cache_dir, capture_output=True, check=False)
    if remote.returncode == 0:
        if remote.stdout.strip() != remote_url:
            run(["git", "remote", "set-url", remote_name, remote_url], cwd=cache_dir)
        return
    run(["git", "remote", "add", remote_name, remote_url], cwd=cache_dir)


def branch_url(fork_url: str, branch: str) -> str:
    return f"{fork_url.rstrip('/')}/tree/{quote(branch, safe='/')}"


def compare_url(fork_url: str, base: str, head: str) -> str:
    return f"{fork_url.rstrip('/')}/compare/{quote(base, safe='') }...{quote(head, safe='/:')}"


def cmd_publish(
    tasks: dict[str, dict[str, Any]],
    task_id: str,
    root: Path,
    *,
    evaluated_agent: str,
    run_id: str,
) -> int:
    task = tasks[task_id]
    run_path = resolve_run_dir(root, task_id, evaluated_agent, run_id)
    blind_manifest = load_json(run_path / "blind_manifest.json")
    snapshot_commits = blind_manifest["snapshot_commits"]
    fork = ensure_fork_repo(task)
    cache_dir = ensure_repo_cache(root, task)
    remote_name = f"{DEFAULT_FORK_OWNER}-fork"
    ensure_remote(cache_dir, remote_name, fork["git_url"])
    branches = branch_names(task_id, evaluated_agent, run_id)
    for condition in SUPPORTED_CONDITIONS:
        worktree = worktree_dir(root, task_id, evaluated_agent, run_id, condition)
        if not worktree.exists():
            raise SystemExit(f"Worktree missing for {condition}: {worktree}")
        run(
            [
                "git",
                "push",
                "--force",
                remote_name,
                f"{snapshot_commits[condition]}:refs/heads/{branches[condition]}",
            ],
            cwd=worktree,
        )
    fork_url = fork["url"]
    publish_meta = {
        "published": True,
        "published_at": datetime.now(timezone.utc).isoformat(),
        "fork": fork,
        "upstream_repo": task["repo"]["name"],
        "ground_truth": task["ground_truth"],
        "branches": {
            condition: {
                "name": branches[condition],
                "url": branch_url(fork_url, branches[condition]),
                "commit": snapshot_commits[condition],
            }
            for condition in SUPPORTED_CONDITIONS
        },
        "compare_urls": {
            "control_vs_pre_fix": compare_url(
                fork_url,
                task["ground_truth"]["pre_fix_commit"],
                branches["control"],
            ),
            "tg_vs_pre_fix": compare_url(
                fork_url,
                task["ground_truth"]["pre_fix_commit"],
                branches["tg"],
            ),
            "control_vs_tg": compare_url(
                fork_url,
                branches["control"],
                branches["tg"],
            ),
        },
        "warning": "Public branch publishing can contaminate future benchmarks. These branches should only be created after the run is complete.",
    }
    write_json(run_path / "publish.json", publish_meta)
    manifest = load_run_manifest(run_path)
    manifest["publish"]["published"] = True
    manifest["publish"]["published_at"] = publish_meta["published_at"]
    write_run_manifest(run_path, manifest)
    judgment = load_json(run_path / "judgment.json")
    report_path = render_report_for_run(
        task=task,
        evaluated_agent=evaluated_agent,
        judge_agent=judgment["judge_agent"],
        root=root,
        run_path=run_path,
    )
    print(f"published {task_id} run {run_id}")
    print(f"report: {report_path}")
    return 0


def cmd_report(
    tasks: dict[str, dict[str, Any]],
    task_id: str,
    root: Path,
    *,
    evaluated_agent: str,
    run_id: str,
) -> int:
    task = tasks[task_id]
    run_path = resolve_run_dir(root, task_id, evaluated_agent, run_id)
    judgment = load_json(run_path / "judgment.json")
    report_path = render_report_for_run(
        task=task,
        evaluated_agent=evaluated_agent,
        judge_agent=judgment["judge_agent"],
        root=root,
        run_path=run_path,
    )
    print(f"report: {report_path}")
    return 0


def cmd_report_all(
    tasks: dict[str, dict[str, Any]],
    task_ids: list[str],
    root: Path,
    *,
    evaluated_agent: str,
) -> int:
    selected = task_ids or list(tasks.keys())
    entries = []
    for task_id in selected:
        task = tasks[task_id]
        run_path = latest_run_dir(root, task_id, evaluated_agent, require_judgment=True)
        judgment = load_json(run_path / "judgment.json")
        report_path = render_report_for_run(
            task=task,
            evaluated_agent=evaluated_agent,
            judge_agent=judgment["judge_agent"],
            root=root,
            run_path=run_path,
        )
        blind_manifest = load_json(run_path / "blind_manifest.json")
        publish_meta = None
        publish_path = run_path / "publish.json"
        if publish_path.exists():
            maybe = normalize_publish_metadata(load_json(publish_path))
            if maybe and maybe.get("published"):
                publish_meta = maybe
        entries.append(
            build_matrix_entry(
                task=task,
                evaluated_agent=evaluated_agent,
                judge_agent=judgment["judge_agent"],
                run_id=run_path.name,
                judgment=judgment,
                blind_manifest=blind_manifest,
                publish_meta=publish_meta,
                report_path=report_path,
                root=root,
            )
        )
    reports = reports_dir(root)
    markdown = build_matrix_markdown(entries)
    write_text(reports / "matrix.md", markdown)
    write_json(reports / "matrix.json", {"entries": entries})
    print(f"matrix: {reports / 'matrix.md'}")
    return 0


def cmd_run_task(
    tasks: dict[str, dict[str, Any]],
    task_id: str,
    root: Path,
    *,
    evaluated_agent: str,
    agent_model: str | None,
    judge_agent: str,
    judge_model: str | None,
    force: bool,
    extra_args: list[str],
) -> int:
    task = tasks[task_id]
    build_args = forwarded_build_args(extra_args, agent_model)
    run_id = new_run_id()

    print(f"[1/6] creating run {run_id} for {task_id}")
    create_run(
        root,
        task,
        evaluated_agent=evaluated_agent,
        run_id=run_id,
        agent_model=agent_model,
        build_args=build_args,
        force=force,
    )
    prepare_run(root, task, evaluated_agent=evaluated_agent, run_id=run_id, force=force)
    print(f"run-id: {run_id}")

    for index, condition in enumerate(SUPPORTED_CONDITIONS, start=2):
        print(f"[{index}/6] launching {evaluated_agent} {condition}")
        result = cmd_launch(
            tasks,
            task_id,
            root,
            agent=evaluated_agent,
            run_id=run_id,
            condition=condition,
            extra_args=build_args,
        )
        if result != 0:
            print(f"stopping after {condition} launch failed with exit code {result}")
            return result

    print(f"[4/6] judging {task_id}")
    result = cmd_judge(
        tasks,
        task_id,
        root,
        evaluated_agent=evaluated_agent,
        run_id=run_id,
        judge_agent=judge_agent,
        judge_model=judge_model,
    )
    if result != 0:
        return result

    print(f"[5/6] publishing {task_id}")
    result = cmd_publish(
        tasks,
        task_id,
        root,
        evaluated_agent=evaluated_agent,
        run_id=run_id,
    )
    if result != 0:
        return result

    print(f"[6/6] refreshing report {task_id}")
    return cmd_report(
        tasks,
        task_id,
        root,
        evaluated_agent=evaluated_agent,
        run_id=run_id,
    )


def build_parser(task_ids: list[str]) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Historical benchmark harness for codex/claude CLI.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("list", help="List available benchmark tasks.")
    list_runs_parser = subparsers.add_parser("list-runs", help="List existing benchmark runs.")
    list_runs_parser.add_argument("task_ids", nargs="*", choices=task_ids)
    list_runs_parser.add_argument("--agent", choices=SUPPORTED_AGENTS)
    list_runs_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)

    show_parser = subparsers.add_parser("show", help="Show details for one task.")
    show_parser.add_argument("task_id", choices=task_ids)

    launch_parser = subparsers.add_parser("launch", help="Launch one prepared variant for an existing run.")
    launch_parser.add_argument("task_id", choices=task_ids)
    launch_parser.add_argument("--agent", required=True, choices=SUPPORTED_AGENTS)
    launch_parser.add_argument("--run-id", required=True)
    launch_parser.add_argument("--variant", required=True, choices=SUPPORTED_CONDITIONS)
    launch_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)

    judge_parser = subparsers.add_parser("judge", help="Blind-judge one run against the human PR.")
    judge_parser.add_argument("task_id", choices=task_ids)
    judge_parser.add_argument("--agent", required=True, choices=SUPPORTED_AGENTS)
    judge_parser.add_argument("--run-id", required=True)
    judge_parser.add_argument("--judge-agent", choices=SUPPORTED_AGENTS, default=None)
    judge_parser.add_argument("--judge-model")
    judge_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)

    publish_parser = subparsers.add_parser("publish", help="Publish both variant branches for one run.")
    publish_parser.add_argument("task_id", choices=task_ids)
    publish_parser.add_argument("--agent", required=True, choices=SUPPORTED_AGENTS)
    publish_parser.add_argument("--run-id", required=True)
    publish_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)

    report_parser = subparsers.add_parser("report", help="Render the markdown report for one run.")
    report_parser.add_argument("task_id", choices=task_ids)
    report_parser.add_argument("--agent", required=True, choices=SUPPORTED_AGENTS)
    report_parser.add_argument("--run-id", required=True)
    report_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)

    report_all_parser = subparsers.add_parser("report-all", help="Render an aggregate markdown matrix from the latest judged runs.")
    report_all_parser.add_argument("task_ids", nargs="*", choices=task_ids)
    report_all_parser.add_argument("--agent", required=True, choices=SUPPORTED_AGENTS)
    report_all_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)

    run_task_parser = subparsers.add_parser(
        "run-task",
        help="Run the full task flow: create a run, launch control, launch tg, judge, publish, and report.",
    )
    run_task_parser.add_argument("task_id", choices=task_ids)
    run_task_parser.add_argument("--agent", required=True, choices=SUPPORTED_AGENTS)
    run_task_parser.add_argument("--agent-model")
    run_task_parser.add_argument("--judge-agent", choices=SUPPORTED_AGENTS, default=None)
    run_task_parser.add_argument("--judge-model")
    run_task_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    run_task_parser.add_argument("--force", action="store_true", help="Overwrite an existing generated run if the run ID collides.")

    return parser


def main() -> int:
    tasks = load_tasks()
    parser = build_parser(sorted(tasks))
    args, extra_args = parser.parse_known_args()

    if args.command == "list":
        return cmd_list(tasks)
    if args.command == "list-runs":
        return cmd_list_runs(tasks, args.task_ids, args.root, evaluated_agent=args.agent)
    if args.command == "show":
        return cmd_show(tasks, args.task_id)
    if args.command == "launch":
        if extra_args and extra_args[0] == "--":
            extra_args = extra_args[1:]
        return cmd_launch(
            tasks,
            args.task_id,
            args.root,
            agent=args.agent,
            run_id=args.run_id,
            condition=args.variant,
            extra_args=extra_args,
        )
    if args.command == "judge":
        return cmd_judge(
            tasks,
            args.task_id,
            args.root,
            evaluated_agent=args.agent,
            run_id=args.run_id,
            judge_agent=args.judge_agent or default_judge_agent(),
            judge_model=args.judge_model,
        )
    if args.command == "publish":
        return cmd_publish(
            tasks,
            args.task_id,
            args.root,
            evaluated_agent=args.agent,
            run_id=args.run_id,
        )
    if args.command == "report":
        return cmd_report(
            tasks,
            args.task_id,
            args.root,
            evaluated_agent=args.agent,
            run_id=args.run_id,
        )
    if args.command == "report-all":
        return cmd_report_all(
            tasks,
            args.task_ids,
            args.root,
            evaluated_agent=args.agent,
        )
    if args.command == "run-task":
        if extra_args and extra_args[0] == "--":
            extra_args = extra_args[1:]
        return cmd_run_task(
            tasks,
            args.task_id,
            args.root,
            evaluated_agent=args.agent,
            agent_model=args.agent_model,
            judge_agent=args.judge_agent or default_judge_agent(),
            judge_model=args.judge_model,
            force=args.force,
            extra_args=extra_args,
        )
    raise AssertionError(f"unknown command: {args.command}")


if __name__ == "__main__":
    sys.exit(main())
