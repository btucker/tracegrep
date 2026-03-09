#!/usr/bin/env python3

import json
import re
import sys


RG_PATTERN = re.compile(r"(^|[;&|]\s*)(rg|grep)\b")
GIT_GREP_PATTERN = re.compile(r"(^|[;&|]\s*)git\s+grep\b")


def deny(reason: str, additional_context: str) -> None:
    print(
        json.dumps(
            {
                "additional_context": additional_context,
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": reason,
                    "additionalContext": additional_context,
                },
            }
        )
    )


def main() -> int:
    payload = json.load(sys.stdin)
    tool_name = payload.get("tool_name")

    if tool_name == "Grep":
        deny(
            "Use tracegrep (`tg`) instead of the Grep tool in this repository.",
            "This repository ships `tg`, which adds Rust call-graph context. "
            "Rerun the search with the Bash tool, for example `tg '<pattern>'`. "
            "Use `tg --json` only when structured output is required.",
        )
        return 0

    if tool_name != "Bash":
        return 0

    command = payload.get("tool_input", {}).get("command", "").strip()
    if not command:
        return 0

    if RG_PATTERN.search(command) or GIT_GREP_PATTERN.search(command):
        deny(
            "Use `tg` instead of `rg` or `grep` for repository search in this repository.",
            "Prefer `tg '<pattern>'` for normal exploration. "
            "Switch to `tg --json '<pattern>'` only when you need structured output.",
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
