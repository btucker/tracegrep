#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
HOOK_NAME="${1:-}"

case "${HOOK_NAME}" in
    session-start)
        exec "${SCRIPT_DIR}/session-start"
        ;;
    pre-tool-use)
        exec python3 "${SCRIPT_DIR}/pre-tool-use.py"
        ;;
    post-tool-use)
        exec python3 "${SCRIPT_DIR}/post-tool-use.py"
        ;;
    *)
        echo "unknown hook: ${HOOK_NAME}" >&2
        exit 1
        ;;
esac
