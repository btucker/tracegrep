#!/usr/bin/env python3

import json
import shlex
import sys


def suggest(additional_context: str) -> None:
    """Emit guidance (not a hard block) suggesting tg instead."""
    print(json.dumps({"additional_context": additional_context}))


def _first_command(command: str) -> str:
    """Return the first real token of *command*, skipping env assignments."""
    try:
        tokens = shlex.split(command)
    except ValueError:
        return ""
    for tok in tokens:
        if "=" in tok:
            continue
        return tok
    return ""


def main() -> int:
    payload = json.load(sys.stdin)
    tool_name = payload.get("tool_name")

    if tool_name != "Bash":
        return 0

    command = payload.get("tool_input", {}).get("command", "").strip()
    if not command:
        return 0

    # Only suggest tg when rg/grep/git-grep is the *primary* command.
    # Piped grep (e.g. `ls | grep foo`) and grep in string args are fine.
    first_cmd = _first_command(command)
    if first_cmd in ("rg", "grep"):
        suggest(
            "Prefer `tg '<pattern>'` for normal exploration. "
            "Switch to `tg --json '<pattern>'` only when you need structured output.",
        )
    elif first_cmd == "git":
        # Check if 'grep' appears as a git subcommand (before any pipe)
        before_pipe = command.split("|")[0]
        try:
            tokens = shlex.split(before_pipe)
        except ValueError:
            tokens = before_pipe.split()
        if len(tokens) >= 2 and tokens[1] == "grep":
            suggest(
                "Prefer `tg '<pattern>'` for normal exploration. "
                "Switch to `tg --json '<pattern>'` only when you need structured output.",
            )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
