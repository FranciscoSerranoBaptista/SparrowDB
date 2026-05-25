---
skill: debugging
type: workflow
trigger: >
  Use when a SparrowDB instance, query, or build is behaving
  unexpectedly — compile errors, HTTP error responses, wrong query
  results, performance issues, or async hangs.
entry_point: "Step 1 — Classify the symptom"
exits:
  - setup.md      # if the instance is not running at all
  - migration.md  # if a schema change caused the regression
related:
  - docs/HTTP_API.md
  - docs/auth.md
  - CLAUDE.md
---

# SparrowDB — Debugging Workflow

---

## Step 1 — Classify the symptom

```
A — Compile / build error
    HQL compiler error (ariadne output), Cargo build failure

B — Runtime HTTP error
    HTTP response with non-2xx status and a JSON "code" field

C — Wrong results
    Query returns unexpected data, missing nodes, wrong shape

D — Performance
    Slow queries, high latency, timeouts under load

E — Async hang / deadlock
    Process stops responding; Tokio runtime appears blocked
```

---

## Step 2 — Run baseline checks

Always run these first regardless of symptom:

```bash
# Is the instance healthy?
curl -H "x-api-key: $TOKEN" http://localhost:6969/diagnostics

# Does the schema look right?
curl -H "x-api-key: $TOKEN" http://localhost:6969/introspect
```

`/diagnostics` returns:
```json
{
  "nodes": <count>,
  "edges": <count>,
  "vectors": {
    "total": <count>,
    "active": <count>,
    "soft_deleted": <count>,
    "hnsw_edges": <count>,
    "entry_point_present": <bool>
  }
}
```

Note: high `soft_deleted` count indicates HNSW index degradation (see **Symptom C / D**).

---

## Step 3 — Isolate with runtime eval

Enable the dynamic eval endpoint to test a query without a compiled endpoint:

```bash
# Start instance with runtime eval enabled
SPARROW_RUNTIME_HQL=1 sparrow run

# Send a raw HQL statement
curl -X POST http://localhost:6969/__hql_runtime_eval \
     -H "Content-Type: application/json" \
     -H "x-api-key: $TOKEN" \
     -d '{"query": "N<User>(\"some-id\") RETURN _"}'
```

Use this to reproduce issues with the smallest possible query before looking at
complex multi-step queries.

---

## Step 4 — Branch on symptom

### Symptom A — Compile / build error

```
1. Read the ariadne error output carefully:
     → file path : line : column with underline and note
     → the note tells you what the compiler expected

2. Check feature flags — tests need both storage and server:
     cargo test --features lmdb,server

3. If the compiler feature itself fails to compile:
     → ensure the ariadne crate is present in sparrow-core/Cargo.toml
     → ariadne MUST be included when the `compiler` feature is active

4. Grammar / syntax error in .hx file:
     → match your syntax against docs/HQL.md
     → the PEG grammar lives at crates/sparrow-core/src/grammar.pest
```

---

### Symptom B — Runtime HTTP error

| Error code | HTTP | Cause | Fix |
|------------|------|-------|-----|
| `INVALID_API_KEY` | 401 | Missing or wrong `x-api-key` header | Check header; see `docs/auth.md` |
| `FORBIDDEN` | 403 | Token role is too low | Use `admin` or `read_write` role token |
| `NOT_FOUND` (query) | 404 | Query name not registered | Check `/introspect` for registered route names; case-sensitive |
| `NOT_FOUND` (v1) | 404 | `/v1/query` traffic hitting wildcard handler | The `/v1/query` route MUST be registered before the `/{*path}` wildcard |
| `GRAPH_ERROR` | 500 | Storage-level failure | Check if `write_txn()` was called outside the WorkerPool writer thread |
| `VECTOR_ERROR` | 500 | HNSW / embedding failure | Check vector dimension mismatch; check `soft_deleted` count |

---

### Symptom C — Wrong results

```
Missing or wrong nodes:
  → Check WHERE predicate logic — AND binds tighter than OR
  → Check edge direction — Out<E> vs In<E>; FromN vs ToN

Wrong return shape:
  → Check field remapping — !{fields} excludes from response only,
    not from storage; spread .. may be including unexpected fields

Stale vector results (ghost neighbours in similarity search):
  → DROP on a node soft-deletes its HNSW entry but does NOT compact
  → Stale entries accumulate and degrade recall over time
  → Check: high `soft_deleted` in /diagnostics confirms this
  → Fix: re-embed the collection into a fresh vector type
         (no in-place compaction is currently available)

Result count mismatch:
  → Check RANGE/FIRST operators — may be slicing the result set
  → Check GROUP_BY — changes the shape of results
```

---

### Symptom D — Performance

```
1. Run a load test to measure baseline:
     sparrow stress <instance>

2. Check /diagnostics for HNSW health:
     → high soft_deleted / total ratio → index degraded
     → entry_point_present = false → HNSW is empty or corrupted

3. Write throughput bottleneck:
     → All writes serialise through the single WorkerPool writer
     → Batch writes where possible (BatchAddV for vectors)
     → Consider whether RocksDB backend fits your workload better
       (check crates/sparrow-core/Cargo.toml for current backend options)

4. Slow queries:
     → Add WHERE filters early in the traversal chain to prune the graph
     → Use indexed fields (INDEX, UNIQUE INDEX) in WHERE predicates
     → Vector search k value: smaller k = faster but less recall
```

---

### Symptom E — Async hang / deadlock

**Most common cause: `std::process::Command` inside an async function.**

```
# Wrong — blocks the Tokio thread pool:
let output = std::process::Command::new("docker").status()?;

# Correct:
let output = tokio::process::Command::new("docker").status().await?;
```

Search for the violation:
```bash
grep -rn 'std::process::Command' crates/ --include='*.rs'
```

Any hit inside an `async fn` is a bug. Replace with `tokio::process::Command`.

**Second most common: write transaction held across an await point.**
LMDB write locks must not cross `.await` boundaries. A `write_txn()` must be
acquired, used, and committed/aborted within a single synchronous block.

```bash
# Enable Tokio tracing to find blocked tasks:
RUST_LOG=tokio=trace sparrow run 2>&1 | grep -i 'block\|park\|poll'
```

---

## Enabling Debug Output

Build sparrow-core with the `debug-output` feature for verbose macro expansion
diagnostics (prints generated Rust code during compilation):

```bash
cargo build -p sparrow-core --features lmdb,server,debug-output
```

Enable runtime logging:
```bash
RUST_LOG=sparrow_db=debug sparrow run
```

---

## Log Streaming

Stream logs from a running Docker instance:

```bash
sparrow logs <instance>
```

Example:
```bash
sparrow logs dev
```

---

## Dev-Only Debug Endpoints

Available when the instance is built with the `dev-instance` feature flag.
Endpoint paths use hyphens (verified against `crates/sparrow-core/src/sparrow_gateway/gateway.rs`):

```bash
# Fetch a specific node by ID
curl -X GET -H "x-api-key: $TOKEN" \
     "http://localhost:6969/node-details?id=<node-id>"

# List all nodes of a type
curl -X GET -H "x-api-key: $TOKEN" \
     "http://localhost:6969/nodes-by-label?label=User"

# Get edges and neighbours of a node
curl -X GET -H "x-api-key: $TOKEN" \
     "http://localhost:6969/node-connections?id=<node-id>"
```

These endpoints are not available in production builds (`production` feature).

---

## Known HNSW Caveats

- **Soft delete accumulation**: `DROP` on a node marks its HNSW vector as inactive
  but does not remove its graph edges from the index. Over time, soft-deleted entries
  degrade recall precision. Monitor `soft_deleted` in `/diagnostics`.
- **No hard delete / compaction**: currently unavailable. Mitigation: re-embed the
  collection into a fresh vector type after heavy deletion.
- **`entry_point_present: false`**: the HNSW graph has no entry point — the vector
  collection is empty or was never populated.

---

## Test Isolation

LMDB stress tests must be run with a single thread to avoid write transaction
conflicts:

```bash
cargo test --package sparrow-core --features lmdb -- --test-threads=1
```

Tests marked with `#[serial]` (from the `serial_test` crate) enforce this
automatically when run through the normal test harness with `--test-threads=1`.

---

*HTTP error codes → `docs/HTTP_API.md`*
*Auth flow → `docs/auth.md`*
*Setup from scratch → `docs/skills/setup.md`*
*Schema migrations → `docs/skills/migration.md`*
