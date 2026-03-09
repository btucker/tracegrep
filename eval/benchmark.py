#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# ///

from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path
from typing import Any


SCRIPT_DIR = Path(__file__).resolve().parent
TASKS_PATH = SCRIPT_DIR / "tasks.json"
DEFAULT_ROOT = SCRIPT_DIR / "workspaces"
SUPPORTED_AGENTS = ("codex", "claude")
SUPPORTED_CONDITIONS = ("control", "tg")
TRACEGREP_SKILL_SOURCE = SCRIPT_DIR.parent / "skills" / "tracegrep"


def load_tasks() -> dict[str, dict[str, Any]]:
    tasks = json.loads(TASKS_PATH.read_text())
    return {task["id"]: task for task in tasks}


def repo_slug(task: dict[str, Any]) -> str:
    return task["repo"]["name"].replace("/", "__")


def cache_repo_dir(root: Path, task: dict[str, Any]) -> Path:
    return root / "cache" / repo_slug(task)


def run_dir(root: Path, task_id: str) -> Path:
    return root / "runs" / task_id


def prompt_path(root: Path, task_id: str, condition: str) -> Path:
    return run_dir(root, task_id) / "prompts" / f"{condition}.md"


def worktree_dir(root: Path, task_id: str, condition: str) -> Path:
    return run_dir(root, task_id) / "worktrees" / condition


def launch_script_path(root: Path, task_id: str, agent: str, condition: str) -> Path:
    return run_dir(root, task_id) / f"launch_{agent}_{condition}.sh"


def run(
    args: list[str],
    *,
    cwd: Path | None = None,
    capture_output: bool = False,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=str(cwd) if cwd else None,
        check=True,
        text=True,
        capture_output=capture_output,
    )


def ensure_root(root: Path) -> None:
    (root / "cache").mkdir(parents=True, exist_ok=True)
    (root / "runs").mkdir(parents=True, exist_ok=True)


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
    condition: str,
    *,
    force: bool,
) -> Path:
    cache_dir = ensure_repo_cache(root, task)
    path = worktree_dir(root, task["id"], condition)
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

    if condition == "tg":
        if not TRACEGREP_SKILL_SOURCE.exists():
            raise SystemExit(f"tracegrep skill source not found at {TRACEGREP_SKILL_SOURCE}")
        remove_tree(codex_skill_dir)
        codex_skill_dir.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(TRACEGREP_SKILL_SOURCE, codex_skill_dir)
        claude_settings_path.parent.mkdir(parents=True, exist_ok=True)
        write_claude_settings(claude_settings_path)
        return

    remove_tree(codex_skill_dir)
    if claude_settings_path.exists():
        claude_settings_path.unlink()


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
    focus = "\n".join(f"- {item}" for item in task["evaluation_focus"])
    parts = [
        "# Benchmark Task",
        f"Task: {task['prompt']['title']}",
        body,
        textwrap.dedent(
            """\
            Constraints:
            - Work only from the checked-out repository state and this prompt.
            - Do not browse the web, open GitHub issues or PRs, or inspect repository history beyond the current checkout.
            - Favor reuse of existing abstractions and patterns over adding parallel implementations.
            - Run relevant tests or checks before finishing, and mention what you ran.
            """
        ).strip(),
        condition_search_guidance(condition),
        "What to optimize for:\n" + focus,
    ]
    return "\n\n".join(parts) + "\n"


def build_run_readme(task: dict[str, Any], root: Path) -> str:
    base = run_dir(root, task["id"])
    lines = [
        f"# {task['id']}",
        "",
        f"- Repo: {task['repo']['name']}",
        f"- License: {task['repo']['license']}",
        f"- Language: {task['repo']['language']}",
        f"- Issue: {task['issue']['url']}",
        f"- Hidden PR ground truth: {task['ground_truth']['pr_url']}",
        "",
        "Launchers:",
    ]
    for agent in SUPPORTED_AGENTS:
        for condition in SUPPORTED_CONDITIONS:
            script = launch_script_path(root, task["id"], agent, condition)
            lines.append(f"- {script.name}")
    lines.extend(
        [
            "",
            "Prompts:",
            f"- {prompt_path(root, task['id'], 'control')}",
            f"- {prompt_path(root, task['id'], 'tg')}",
            "",
            "Worktrees:",
            f"- {worktree_dir(root, task['id'], 'control')}",
            f"- {worktree_dir(root, task['id'], 'tg')}",
            "",
            "tg condition environment additions:",
            "- `.codex/skills/tracegrep/` copied from this repo",
            "- `.claude/settings.local.json` enabling `tracegrep@tracegrep-dev`",
        ]
    )
    return "\n".join(lines) + "\n"


def build_launcher_script(task: dict[str, Any], root: Path, agent: str, condition: str) -> str:
    base = run_dir(root, task["id"])
    prompt_file = base / "prompts" / f"{condition}.md"
    worktree = base / "worktrees" / condition
    if agent == "codex":
        command = (
            'exec codex --sandbox workspace-write --ask-for-approval never "$@" '
            '"$(cat "$PROMPT_FILE")"'
        )
    elif agent == "claude":
        command = 'exec claude "$@" "$(cat "$PROMPT_FILE")"'
    else:
        raise ValueError(f"unsupported agent: {agent}")
    preflight = ""
    if agent == "claude" and condition == "tg":
        preflight = textwrap.dedent(
            """\
            PLUGIN_ID="tracegrep@tracegrep-dev"
            if ! claude plugin list --json | python3 -c '
            import json
            import sys

            plugin_id = sys.argv[1]
            plugins = json.load(sys.stdin)
            installed = any(plugin.get("id") == plugin_id for plugin in plugins)
            sys.exit(0 if installed else 1)
            ' "$PLUGIN_ID"; then
              echo "Required Claude plugin not installed: $PLUGIN_ID" >&2
              echo "Run this once from the worktree to install it:" >&2
              echo "  claude plugin install $PLUGIN_ID" >&2
              exit 1
            fi

            """
        )
    return textwrap.dedent(
        f"""\
        #!/usr/bin/env bash
        set -euo pipefail

        BASE_DIR="$(cd "$(dirname "${{BASH_SOURCE[0]}}")" && pwd)"
        WORKTREE="{worktree}"
        PROMPT_FILE="{prompt_file}"

        cd "$WORKTREE"
        {preflight}\
        {command}
        """
    )


def write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")


def prepare_task(root: Path, task: dict[str, Any], *, force: bool) -> None:
    ensure_root(root)
    base = run_dir(root, task["id"])
    (base / "prompts").mkdir(parents=True, exist_ok=True)
    (base / "hidden").mkdir(parents=True, exist_ok=True)

    for condition in SUPPORTED_CONDITIONS:
        worktree = ensure_worktree(root, task, condition, force=force)
        configure_condition_environment(worktree, condition)
        prompt = build_prompt(task, condition)
        prompt_path(root, task["id"], condition).write_text(prompt)

    hidden_payload = {
        "task_id": task["id"],
        "repo": task["repo"],
        "issue": task["issue"],
        "ground_truth": task["ground_truth"],
        "evaluation_focus": task["evaluation_focus"],
    }
    write_json(base / "hidden" / "ground_truth.json", hidden_payload)
    (base / "README.md").write_text(build_run_readme(task, root))

    for agent in SUPPORTED_AGENTS:
        for condition in SUPPORTED_CONDITIONS:
            script_path = launch_script_path(root, task["id"], agent, condition)
            script_path.write_text(build_launcher_script(task, root, agent, condition))
            os.chmod(script_path, 0o755)


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


def cmd_prepare(tasks: dict[str, dict[str, Any]], task_ids: list[str], root: Path, force: bool) -> int:
    selected = task_ids or list(tasks.keys())
    for task_id in selected:
        prepare_task(root, tasks[task_id], force=force)
        print(f"prepared {task_id} at {run_dir(root, task_id)}")
    return 0


def cmd_launch(
    tasks: dict[str, dict[str, Any]],
    task_id: str,
    root: Path,
    *,
    agent: str,
    condition: str,
    prepare: bool,
    force: bool,
    extra_args: list[str],
) -> int:
    task = tasks[task_id]
    if prepare:
        prepare_task(root, task, force=force)
    script = launch_script_path(root, task_id, agent, condition)
    if not script.exists():
        raise SystemExit(
            f"{script} does not exist. Run `uv run eval/benchmark.py prepare {task_id}` first."
        )
    command = [str(script), *extra_args]
    print("launching:", " ".join(shlex.quote(part) for part in command))
    completed = subprocess.run(command)
    return completed.returncode


def build_parser(task_ids: list[str]) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Historical benchmark harness for codex/claude CLI.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("list", help="List available benchmark tasks.")

    show_parser = subparsers.add_parser("show", help="Show details for one task.")
    show_parser.add_argument("task_id", choices=task_ids)

    prepare_parser = subparsers.add_parser("prepare", help="Prepare one or more tasks.")
    prepare_parser.add_argument("task_ids", nargs="*", choices=task_ids)
    prepare_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    prepare_parser.add_argument("--force", action="store_true", help="Recreate existing generated worktrees.")

    launch_parser = subparsers.add_parser("launch", help="Launch a prepared task in codex or claude.")
    launch_parser.add_argument("task_id", choices=task_ids)
    launch_parser.add_argument("--agent", required=True, choices=SUPPORTED_AGENTS)
    launch_parser.add_argument("--condition", required=True, choices=SUPPORTED_CONDITIONS)
    launch_parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    launch_parser.add_argument(
        "--prepare",
        action="store_true",
        help="Prepare the task before launching if needed.",
    )
    launch_parser.add_argument("--force", action="store_true", help="Recreate generated worktrees during prepare.")

    return parser


def main() -> int:
    tasks = load_tasks()
    parser = build_parser(sorted(tasks))
    args, extra_args = parser.parse_known_args()

    if args.command == "list":
        return cmd_list(tasks)
    if args.command == "show":
        return cmd_show(tasks, args.task_id)
    if args.command == "prepare":
        return cmd_prepare(tasks, args.task_ids, args.root, args.force)
    if args.command == "launch":
        if extra_args and extra_args[0] == "--":
            extra_args = extra_args[1:]
        return cmd_launch(
            tasks,
            args.task_id,
            args.root,
            agent=args.agent,
            condition=args.condition,
            prepare=args.prepare,
            force=args.force,
            extra_args=extra_args,
        )
    raise AssertionError(f"unknown command: {args.command}")


if __name__ == "__main__":
    sys.exit(main())
