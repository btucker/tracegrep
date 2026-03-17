#!/bin/sh

payload=$(cat)

if ! printf '%s' "$payload" | jq -e . >/dev/null 2>&1; then
  exit 0
fi

pattern=$(printf '%s' "$payload" | jq -r '.tool_input.pattern // empty')
path=$(printf '%s' "$payload" | jq -r '.tool_input.path // empty')
glob=$(printf '%s' "$payload" | jq -r '.tool_input.glob // empty')
tool_name=$(printf '%s' "$payload" | jq -r '.tool_name // "Search"')

shell_quote() {
  printf '%s' "$1" | jq -Rr '@sh'
}

suggested_command='tg'

if [ -n "$glob" ]; then
  suggested_command="$suggested_command --glob $(shell_quote "$glob")"
fi

if [ -n "$pattern" ]; then
  suggested_command="$suggested_command $(shell_quote "$pattern")"
else
  suggested_command="$suggested_command '<pattern>'"
fi

if [ -n "$path" ]; then
  suggested_command="$suggested_command $(shell_quote "$path")"
fi

reason="The $tool_name tool is disabled by the tracegrep plugin. Use the Bash tool with tg instead (for example: $suggested_command). Do not treat this as a ban on grep inside Bash; only the Grep and Search tools are blocked."

jq -n --arg reason "$reason" '{
  continue: true,
  hookSpecificOutput: {
    hookEventName: "PreToolUse",
    permissionDecision: "deny",
    permissionDecisionReason: $reason
  }
}'
