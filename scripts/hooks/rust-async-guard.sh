#!/usr/bin/env bash
# Fires on PostToolUse for Edit and Write tool calls.
# Warns when std::process::Command appears in a .rs file that also
# has async fn — the most common pattern indicating a Tokio blocker.
# Always exits 0 (advisory only; never blocks tool execution).
set -euo pipefail

input=$(cat)

# Extract file path from tool input JSON
file=$(python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('file_path', ''))
except Exception:
    print('')
" <<< "$input" 2>/dev/null || echo "")

# Only check Rust source files
[[ "$file" == *.rs ]] || exit 0
[[ -f "$file" ]] || exit 0

# Warn if both std::process::Command AND async fn appear in the same file.
# Exception: docker.rs is intentionally sync-only (documented in CLAUDE.md).
if [[ "$file" == *"docker.rs" ]]; then
    exit 0
fi

has_std_cmd=$(grep -c 'std::process::Command' "$file" 2>/dev/null || echo 0)
has_async=$(grep -c 'async fn' "$file" 2>/dev/null || echo 0)

if [[ "$has_std_cmd" -gt 0 && "$has_async" -gt 0 ]]; then
    echo ""
    echo "⚠️  ASYNC GUARD: std::process::Command found alongside async fn in:"
    echo "   $file"
    echo "   In async contexts this blocks the Tokio thread pool."
    echo "   Use tokio::process::Command instead."
    echo "   See: CLAUDE.md §std::process::Command is banned in async code"
    echo "   Occurrences:"
    grep -n 'std::process::Command' "$file" | head -5 | sed 's/^/     /'
    echo ""
fi

exit 0
