#!/usr/bin/env bash
# PostToolUse hook for Edit and Write tool calls.
# Auto-formats .rs files with rustfmt after every edit.
# Always exits 0 (advisory only; never blocks tool execution).
set -euo pipefail

input=$(cat)

file=$(python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('file_path', ''))
except Exception:
    print('')
" <<< "$input" 2>/dev/null || echo "")

[[ "$file" == *.rs ]] || exit 0
[[ -f "$file" ]] || exit 0

output=$(rustfmt "$file" 2>&1) || {
    echo ""
    echo "⚠️  RUSTFMT: could not format $file"
    echo "$output"
    echo ""
}

exit 0
