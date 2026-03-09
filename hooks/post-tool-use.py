#!/usr/bin/env python3

import json
import shutil
import subprocess
import sys


MAX_CONTEXT_CHARS = 20000


def build_tracegrep_command(payload: dict) -> list[str] | None:
    tool_input = payload.get("tool_input", {})
    pattern = tool_input.get("pattern")
    if not isinstance(pattern, str) or not pattern:
        return None

    command: list[str] = []
    if shutil.which("tg"):
        command.append("tg")
    elif shutil.which("tracegrep"):
        command.append("tracegrep")
    elif shutil.which("cargo"):
        command.extend(["cargo", "run", "--"])
    else:
        return None

    cwd = payload.get("cwd")
    if isinstance(cwd, str) and cwd:
        command.extend(["--repo", cwd])

    # Match Claude Code Grep defaults as closely as tracegrep's rg passthrough allows.
    command.extend(["--hidden", "--max-columns", "500"])

    if tool_input.get("multiline") is True:
        command.extend(["-U", "--multiline-dotall"])

    if tool_input.get("-i") is True:
        command.append("-i")

    type_value = tool_input.get("type")
    if isinstance(type_value, str) and type_value:
        command.extend(["--type", type_value])

    for key in ("context", "-C", "-B", "-A"):
        value = tool_input.get(key)
        if isinstance(value, int):
            if key == "context":
                command.extend(["-C", str(value)])
            else:
                command.extend([key, str(value)])
            break

    glob_value = tool_input.get("glob")
    if isinstance(glob_value, str) and glob_value.strip():
        for raw_part in glob_value.split():
            for part in raw_part.split(","):
                part = part.strip()
                if part:
                    command.extend(["--glob", part])

    if tool_input.get("-n") is False:
        command.append("--no-line-number")

    path_value = tool_input.get("path")
    if isinstance(path_value, str) and path_value not in ("", "."):
        command.append(path_value)

    command.append(pattern)
    return command


def shorten(text: str) -> str:
    if len(text) <= MAX_CONTEXT_CHARS:
        return text
    return text[:MAX_CONTEXT_CHARS] + "\n\n[tracegrep output truncated]"


def format_additional_context(output: str, command: list[str]) -> str:
    rendered = " ".join(command)
    return (
        "Tracegrep annotation for the immediately preceding Grep call:\n\n"
        f"Command: {rendered}\n\n"
        "This is a tracegrep rerun of the Grep query with Rust call-graph context added. "
        "Use it as an annotated companion to the Grep results, not as a byte-for-byte replacement.\n\n"
        f"{shorten(output)}"
    )


def run_tracegrep(payload: dict) -> str | None:
    command = build_tracegrep_command(payload)
    if command is None:
        return None

    cwd = payload.get("cwd")
    if not isinstance(cwd, str) or not cwd:
        cwd = "."

    completed = subprocess.run(
        command,
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )

    if completed.returncode not in (0, 1):
        stderr = completed.stderr.strip()
        if stderr:
            return f"tracegrep hook failed: {stderr}"
        return f"tracegrep hook failed with exit code {completed.returncode}"

    output = completed.stdout.strip()
    if not output:
        return "tracegrep found no additional call-graph context for this Grep search."

    return format_additional_context(output, command)


def main() -> int:
    payload = json.load(sys.stdin)
    if payload.get("tool_name") != "Grep":
        return 0

    additional_context = run_tracegrep(payload)
    if not additional_context:
        return 0

    print(
        json.dumps(
            {
                "additional_context": additional_context,
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": additional_context,
                },
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
