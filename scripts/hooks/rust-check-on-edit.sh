#!/usr/bin/env bash
# PostToolUse hook for Edit and Write tool calls.
# Runs cargo check on the affected package after editing a .rs file.
# Always exits 0 (advisory only; never blocks tool execution).
set -euo pipefail

WORKSPACE_ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel 2>/dev/null || echo "")
[[ -n "$WORKSPACE_ROOT" ]] || exit 0

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

pkg_name=$(python3 -c "
import sys, os, re

def find_package_name(start_dir, root):
    d = start_dir
    while d and d != root and d != os.path.dirname(d):
        candidate = os.path.join(d, 'Cargo.toml')
        if os.path.exists(candidate):
            with open(candidate) as f:
                content = f.read()
            in_package = False
            for line in content.splitlines():
                if line.strip() == '[package]':
                    in_package = True
                elif line.strip().startswith('[') and in_package:
                    break
                elif in_package:
                    m = re.match(r'^\s*name\s*=\s*\"([^\"]+)\"', line)
                    if m:
                        print(m.group(1))
                        sys.exit(0)
        d = os.path.dirname(d)
    print('')

file_path = sys.stdin.read().strip()
start_dir = os.path.dirname(os.path.abspath(file_path))
root = '$WORKSPACE_ROOT'
find_package_name(start_dir, root)
" <<< "$file" 2>/dev/null || echo "")

[[ -n "$pkg_name" ]] || exit 0

output=$(CARGO_TERM_COLOR=never timeout 60 cargo check --package "$pkg_name" \
    --manifest-path "$WORKSPACE_ROOT/Cargo.toml" 2>&1) || {
    echo ""
    echo "━━━ CARGO CHECK ERRORS ($pkg_name) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "$output" | grep -E "^error" -A 5 | head -80
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
}

exit 0
