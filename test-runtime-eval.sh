#!/usr/bin/env bash
set -euo pipefail

# ── Config ──────────────────────────────────────────────────────────────────
REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
SPARROW_BIN="$REPO_ROOT/target/debug/sparrow"
CONTAINER_BIN="$REPO_ROOT/target/debug/sparrow-container"
PROJECT_DIR="/tmp/sparrow-runtime-eval-project"
DATA_DIR="/tmp/sparrow-runtime-eval-data"
QUERIES_RS="$REPO_ROOT/sparrow-container/src/queries.rs"
QUERIES_RS_BAK="/tmp/sparrow-runtime-eval-queries.rs.bak"
PORT=7777
SERVER_PID=""

# ── Test counters ─────────────────────────────────────────────────────────────
PASS=0
FAIL=0

# ── Helpers ───────────────────────────────────────────────────────────────────
assert_contains() {
    local label=$1 response=$2 expected=$3
    if echo "$response" | grep -q "$expected"; then
        echo "PASS: $label"
        PASS=$((PASS+1))
    else
        echo "FAIL: $label — expected '$expected' in response"
        echo "  Response: $response"
        FAIL=$((FAIL+1))
    fi
}

assert_not_contains() {
    local label=$1 response=$2 forbidden=$3
    if ! echo "$response" | grep -q "$forbidden"; then
        echo "PASS: $label"
        PASS=$((PASS+1))
    else
        echo "FAIL: $label — '$forbidden' should not be in response"
        echo "  Response: $response"
        FAIL=$((FAIL+1))
    fi
}

eval_query() {
    local query=$1 params=${2:-'{}'}
    curl -s -X POST "http://localhost:$PORT/__hql_runtime_eval" \
      -H "Content-Type: application/json" \
      -d "{\"query\": $(echo "$query" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))'), \"params\": $params}"
}

# ── Cleanup ───────────────────────────────────────────────────────────────────
cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        echo ""
        echo "Stopping server (PID $SERVER_PID)..."
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    if [ -f "$QUERIES_RS_BAK" ]; then
        echo "Restoring original queries.rs..."
        cp "$QUERIES_RS_BAK" "$QUERIES_RS"
        rm -f "$QUERIES_RS_BAK"
    fi
    rm -rf "$PROJECT_DIR" "$DATA_DIR"
}
trap cleanup EXIT INT TERM

# ── Check port is free ────────────────────────────────────────────────────────
if lsof -iTCP:"$PORT" -sTCP:LISTEN -P 2>/dev/null | grep -q "$PORT"; then
    echo "ERROR: Port $PORT is already in use. Kill the process first."
    exit 1
fi

# ── Step 1: Create project ────────────────────────────────────────────────────
echo "=== Step 1: Creating runtime-eval test project ==="
rm -rf "$PROJECT_DIR"
mkdir -p "$PROJECT_DIR"

cat > "$PROJECT_DIR/sparrow.toml" << 'EOF'
[project]
name = "runtime-eval-test"
queries = "."

[local.dev]
port = 7777
build_mode = "dev"

[cloud]
EOF

cat > "$PROJECT_DIR/schema.hx" << 'EOF'
N::People {
    UNIQUE INDEX person_id: String,
    first_name: String,
    last_name: String,
    age: I32
}

N::Company {
    UNIQUE INDEX name: String
}

E::WorksAt UNIQUE {
    From: People,
    To: Company,
    Properties: {}
}
EOF

cat > "$PROJECT_DIR/queries.hx" << 'EOF'
QUERY dummy() =>
    p <- N<People>
RETURN p
EOF

echo "Project created at $PROJECT_DIR"

# ── Step 2: Compile queries ───────────────────────────────────────────────────
echo ""
echo "=== Step 2: Compiling HQL queries ==="

# Back up current queries.rs
cp "$QUERIES_RS" "$QUERIES_RS_BAK"

"$SPARROW_BIN" compile \
    --path "$PROJECT_DIR" \
    --output "$REPO_ROOT/sparrow-container/src/"

echo "Compiled → sparrow-container/src/queries.rs"

# ── Step 3: Build container ───────────────────────────────────────────────────
echo ""
echo "=== Step 3: Building sparrow-container ==="
cargo build -p sparrow-container 2>&1

echo "Built → $CONTAINER_BIN"

# ── Step 4: Start server ──────────────────────────────────────────────────────
echo ""
echo "=== Step 4: Starting SparrowDB on port $PORT with SPARROW_RUNTIME_HQL=true ==="
mkdir -p "$DATA_DIR"

SPARROW_DATA_DIR="$DATA_DIR" SPARROW_PORT="$PORT" SPARROW_RUNTIME_HQL=true "$CONTAINER_BIN" &
SERVER_PID=$!
echo "Server PID: $SERVER_PID"

# Wait for server to be ready (up to 15 seconds)
echo -n "Waiting for server"
for i in $(seq 1 30); do
    if curl -s --max-time 1 "http://localhost:$PORT/dummy" \
            -X POST -H "Content-Type: application/json" -d '{}' > /dev/null 2>&1; then
        echo " ready!"
        break
    fi
    echo -n "."
    sleep 0.5
    if [ "$i" -eq 30 ]; then
        echo ""
        echo "ERROR: Server did not start in time."
        exit 1
    fi
done

# ── Step 5: Run test phases ───────────────────────────────────────────────────
echo ""
echo "=== Step 5: Running HQL runtime eval test phases ==="

# ── Phase 1: Create nodes ─────────────────────────────────────────────────────
echo ""
echo "--- Phase 1: Create nodes ---"

r=$(eval_query "QUERY create(person_id: String, first_name: String, last_name: String, age: I32) =>
  p <- AddN<People>({person_id: person_id, first_name: first_name, last_name: last_name, age: age})
RETURN p" '{"person_id": "p1", "first_name": "Alice", "last_name": "Smith", "age": 30}')
assert_contains "create person" "$r" "Alice"

r=$(eval_query "QUERY createCo(name: String) =>
  c <- AddN<Company>({name: name})
RETURN c" '{"name": "Acme"}')
assert_contains "create company" "$r" "Acme"

# ── Phase 2: Read nodes ───────────────────────────────────────────────────────
echo ""
echo "--- Phase 2: Read nodes ---"

r=$(eval_query "QUERY get(pid: String) =>
  p <- N<People>({person_id: pid})
RETURN p" '{"pid": "p1"}')
assert_contains "lookup by index" "$r" "Alice"

r=$(eval_query "QUERY getAll() =>
  p <- N<People>
RETURN p")
assert_contains "scan all people" "$r" "Alice"

# ── Phase 3: Edge creation and traversal ──────────────────────────────────────
echo ""
echo "--- Phase 3: Edge creation and traversal ---"

r=$(eval_query "QUERY link(pid: String, cname: String) =>
  person <- N<People>({person_id: pid})
  company <- N<Company>({name: cname})
  e <- AddE<WorksAt>()::From(person)::To(company)
RETURN e" '{"pid": "p1", "cname": "Acme"}')
assert_contains "create edge" "$r" "WorksAt"

r=$(eval_query "QUERY getCompany(pid: String) =>
  person <- N<People>({person_id: pid})
  company <- person::Out<WorksAt>
RETURN company" '{"pid": "p1"}')
assert_contains "traverse out edge" "$r" "Acme"

# ── Phase 4: WHERE filter ─────────────────────────────────────────────────────
echo ""
echo "--- Phase 4: WHERE filter ---"

# Create a second person aged 20 so we have two people with different ages
eval_query "QUERY create2(person_id: String, first_name: String, last_name: String, age: I32) =>
  p <- AddN<People>({person_id: person_id, first_name: first_name, last_name: last_name, age: age})
RETURN p" '{"person_id": "p2", "first_name": "Bob", "last_name": "Jones", "age": 20}' > /dev/null

r=$(eval_query "QUERY youngPeople(max_age: I32) =>
  p <- N<People>::WHERE(_::{age}::LT(max_age))
RETURN p" '{"max_age": 25}')
assert_contains "filter finds Bob (age 20)" "$r" "Bob"
assert_not_contains "filter excludes Alice (age 30)" "$r" "Alice"

# ── Phase 5: Delete ───────────────────────────────────────────────────────────
echo ""
echo "--- Phase 5: Delete ---"

eval_query "QUERY del(pid: String) =>
  DROP N<People>({person_id: pid})
RETURN NONE" '{"pid": "p2"}' > /dev/null

r=$(eval_query "QUERY check(pid: String) =>
  p <- N<People>({person_id: pid})
RETURN p" '{"pid": "p2"}')
assert_not_contains "deleted person not found" "$r" "Bob"

# ── Phase 6: Update ───────────────────────────────────────────────────────────
echo ""
echo "--- Phase 6: Update ---"

r=$(eval_query "QUERY updateAge(pid: String, new_age: I32) =>
  p <- N<People>({person_id: pid})::UPDATE({age: new_age})
RETURN p" '{"pid": "p1", "new_age": 35}')
assert_contains "update returns updated node" "$r" "35"

r=$(eval_query "QUERY getAge(pid: String) =>
  p <- N<People>({person_id: pid})
RETURN p" '{"pid": "p1"}')
assert_contains "updated age persisted" "$r" "35"

# ── Phase 7: Error handling ───────────────────────────────────────────────────
echo ""
echo "--- Phase 7: Error handling ---"

r=$(eval_query "QUERY bad() =>
  x <- N<Nonexistent>
RETURN x")
assert_contains "unknown type rejected" "$r" "error"

r=$(eval_query "QUERY syn() => @@@ invalid RETURN x")
assert_contains "syntax error rejected" "$r" "error"

# ── Results ───────────────────────────────────────────────────────────────────
echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

# Stop server before running stress test
echo ""
echo "Stopping server (PID $SERVER_PID)..."
kill "$SERVER_PID" 2>/dev/null || true
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=""

# Wait for port to be freed (up to 10 seconds)
echo -n "Waiting for port $PORT to be free"
for i in $(seq 1 20); do
    if ! lsof -iTCP:"$PORT" -sTCP:LISTEN -P 2>/dev/null | grep -q "$PORT"; then
        echo " free!"
        break
    fi
    echo -n "."
    sleep 0.5
    if [ "$i" -eq 20 ]; then
        echo ""
        echo "ERROR: Port $PORT still in use after 10s"
        exit 1
    fi
done

# Restore queries.rs before running stress test
if [ -f "$QUERIES_RS_BAK" ]; then
    echo "Restoring original queries.rs before stress test..."
    cp "$QUERIES_RS_BAK" "$QUERIES_RS"
    rm -f "$QUERIES_RS_BAK"
fi

# ── Stress test regression ────────────────────────────────────────────────────
echo ""
echo "=== Running stress test regression ==="
"$REPO_ROOT/test-stress.sh" 2>&1

# ── Final exit code ───────────────────────────────────────────────────────────
[ "$FAIL" -eq 0 ] || exit 1
