#!/usr/bin/env bash
# Runs after any cargo build/test/check to sweep stale artifacts.
# Called by Claude Code's PostToolUse hook; also usable standalone.
set -euo pipefail

input=$(cat)
cmd=$(echo "$input" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_input',{}).get('command',''))" 2>/dev/null || echo "")

case "$cmd" in
  *"cargo build"*|*"cargo test"*|*"cargo check"*)
    cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
    cargo sweep -t 3 >/dev/null 2>&1 || true
    ;;
esac

exit 0
