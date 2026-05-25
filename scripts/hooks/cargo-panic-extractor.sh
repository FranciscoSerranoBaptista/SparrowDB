#!/usr/bin/env bash
# Fires on PostToolUse for Bash tool calls.
# When a cargo test or cargo bench command runs, extracts panic/OOM/DB
# error signals from the output and prints a structured summary.
# Always exits 0 (never blocks tool execution).
set -euo pipefail

input=$(cat)

# Extract the command that was run
cmd=$(python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('command', ''))
except Exception:
    print('')
" <<< "$input" 2>/dev/null || echo "")

# Only process cargo test / cargo bench commands
case "$cmd" in
    *"cargo test"*|*"cargo bench"*) ;;
    *) exit 0 ;;
esac

# Extract test output from the tool response
output=$(python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    resp = d.get('tool_response', '')
    if isinstance(resp, dict):
        print(resp.get('output', '') + resp.get('error', ''))
    else:
        print(str(resp))
except Exception:
    print('')
" <<< "$input" 2>/dev/null || echo "")

if [[ -z "$output" ]]; then
    exit 0
fi

# Count signal types
panics=$(echo "$output" | grep -c "panicked at" 2>/dev/null) || panics=0
failures=$(echo "$output" | grep -c "^test .* FAILED$" 2>/dev/null) || failures=0
oom=$(echo "$output" | grep -icE "out of memory|oom|^Killed" 2>/dev/null) || oom=0
db_errors=$(echo "$output" | grep -cE "GRAPH_ERROR|VECTOR_ERROR|LMDB|MDB_" 2>/dev/null) || db_errors=0

# Only print summary if there is something interesting
total=$((panics + failures + oom + db_errors))
if [[ "$total" -eq 0 ]]; then
    exit 0
fi

echo ""
echo "━━━ CARGO SIGNAL SUMMARY ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
printf "  PANICS     : %s\n" "$panics"
printf "  FAILURES   : %s\n" "$failures"
printf "  OOM/KILLED : %s\n" "$oom"
printf "  DB ERRORS  : %s\n" "$db_errors"
echo "━━━ DETAILS ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "$output" | grep -E \
    "panicked at|^test .* FAILED|out of memory|^Killed|GRAPH_ERROR|VECTOR_ERROR|LMDB|MDB_" \
    | head -30
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

exit 0
