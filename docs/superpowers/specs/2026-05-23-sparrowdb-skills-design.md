# SparrowDB Skills Design

**Date**: 2026-05-23  
**Status**: Approved  
**Scope**: Four skill files for AI coding assistants working with SparrowDB

---

## Overview

Four `docs/skills/*.md` files that give AI assistants structured, reliable playbooks and reference material for common SparrowDB workflows. Files are committed to the repo so any developer or agent that clones the project gets them automatically.

Two structural types are used, chosen per file based on nature of the task:

| Type | When to use | AI consumption pattern |
|------|-------------|----------------------|
| **A — Workflow skill** | Sequential process with decision points | Follow steps top-to-bottom; branch on failure modes |
| **C — Reference** | Knowledge needed while writing/reviewing code | Consult specific section; don't execute linearly |

---

## File Layout

```
docs/skills/
  querying.md     ← Type C: reference
  setup.md        ← Type A: workflow skill
  migration.md    ← Type A: workflow skill
  debugging.md    ← Type A: workflow skill
```

---

## Frontmatter Format

Every file opens with a YAML frontmatter block.

**Type C (reference):**
```yaml
---
skill: querying
type: reference
trigger: >
  Use when writing or reviewing HQL queries, understanding query
  results, optimising traversal, or exposing queries as MCP tools.
related:
  - docs/HQL.md
  - docs/HTTP_API.md
---
```

**Type A (workflow) — adds `entry_point` and `exits`:**
```yaml
---
skill: setup
type: workflow
trigger: >
  Use when initialising a new SparrowDB project, configuring an
  instance, or onboarding into an existing project.
entry_point: "Step 1 — Choose setup path"
exits:
  - querying.md   # once the instance is live
  - debugging.md  # if setup fails
related:
  - docs/auth.md
  - docs/HTTP_API.md
---
```

---

## File Designs

### 1. `querying.md` — Type C (reference)

**Purpose**: Everything an AI assistant needs to read and write correct HQL.

**Sections:**

#### 1.1 Concept map
Short prose covering: nodes (`N`), edges (`E`), vectors (`V`), the `::` step-chaining operator, anonymous traversal (`_`), and how a compiled query maps to an HTTP endpoint.

#### 1.2 Query anatomy
The shape of a `QUERY` definition: name, typed params, body statements, `RETURN` clause, and the resulting POST endpoint at `/<QueryName>`.

#### 1.3 Pattern library
Ten named patterns, each with a minimal HQL example and a one-line description:

| Pattern | Key operators |
|---------|--------------|
| Node lookup by ID | `N<Type>(id)` |
| Edge traversal outbound | `Out<EdgeType>` |
| Edge traversal inbound | `In<EdgeType>` |
| Vector similarity search | `SearchV<Type>(vec, k)` |
| Node-field vector search | `SearchN<Type>(vec, k)` |
| BM25 full-text search | `SearchBM25<Type>(text, k)` |
| Hybrid search + rerank | `SearchV` + `SearchBM25` + `RerankRRF` / `RerankMMR` |
| Filtered traversal | `WHERE(predicate)`, `AND`, `OR` |
| Aggregation | `COUNT`, `GROUP_BY`, `ORDER<Asc\|Desc>`, `RANGE` |
| Shortest path | `ShortestPath`, `ShortestPathDijkstras`, `ShortestPathAStar` |

#### 1.4 MCP tool exposure
How to annotate queries for AI agent consumption:
- `#[mcp]` — exposes query as an MCP tool
- `#[model("embedding-model-name")]` — sets model for `Embed(text)` calls
- What the generated MCP tool name and input schema look like

#### 1.5 Type system quick reference
Table: scalar types (`String`, `Boolean`, `F32`–`F64`, `I8`–`I64`, `U8`–`U128`), special types (`ID`, `Date`, `NOW`), complex types (`[T]` arrays, `{fields}` objects, `vector(N)` embeddings). When to use each.

#### 1.6 Gotchas
- `ID` is a UUID type — never pass as `String`
- Vector dimension mismatch: `vector(N)` in schema must match embedding model output dimension exactly
- Soft-deleted vectors: `DROP` on a node marks its vector soft-deleted in the HNSW index but does not compact. Stale entries accumulate; search results may include ghost neighbours until a rebuild
- `UNIQUE INDEX` violations are silent upserts in some operators — know whether you're using `AddN` (errors on dup) vs `UpsertN` (merges)
- Field remapping with `!{fields}` excludes fields from return — doesn't delete them from storage

#### 1.7 Operator quick-reference table
Scannable two-column table: operator → purpose, grouped by category (mutation, traversal, vector, aggregation, math).

---

### 2. `setup.md` — Type A (workflow)

**Purpose**: Get a SparrowDB instance running from zero.

**Steps:**

```
Step 1 — Choose setup path
  ├── AI agent / fastest path → sparrow-chef (Step 1a)
  └── Manual / full control  → sparrow CLI (Step 1b)

Step 1a — Chef path (zero-friction)
  sparrow-chef cook --auto
  → scaffolds project, pulls Docker image, starts DB, seeds example data
  → produces SPARROWDB_CHEF_PROMPT.md for agent context
  → skip to Step 5

Step 1b — CLI path
  cargo install sparrow-cli
  sparrow init <project-name>
  cd <project-name>
  # Creates: sparrow.toml, queries/, .sparrow/ (git-ignored)

Step 2 — Configure sparrow.toml
  [project] name, queries dir
  [local.dev] port, build_mode, storage_backend (lmdb | rocks)

Step 3 — Write schema
  Create queries/<schema>.hx
  Define N::, E::, V:: types with field types and indexes

Step 4 — Start instance
  sparrow run          ← direct (no Docker)
  sparrow push dev     ← compile + deploy via Docker Compose

Step 5 — Seed auth token
  → See docs/auth.md for full token lifecycle
  Short path: set SPARROW_API_KEY=<secret> before start;
  instance auto-seeds an admin token on first boot.
  Fresh instance (no tokens): requests succeed unauthenticated.
  After first token: every request needs x-api-key header.

Step 6 — Verify
  GET /introspect   → schema JSON matches expectations
  GET /diagnostics  → node/edge/vector counts (all zero on fresh DB)
```

**Sections after steps:**

- **Environment variables reference** — full table: `SPARROW_DATA_DIR`, `SPARROW_HOME`, `SPARROW_CACHE_DIR`, `SPARROW_PORT`, `SPARROW_API_KEY`, `SPARROW_RUNTIME_EVAL`
- **Storage backend choice** — LMDB: zero-copy reads, crash-safe, single writer, low latency; RocksDB: high write throughput, compaction, column families. Switch via `storage_backend` in `sparrow.toml`. LMDB is the default and the right choice unless write throughput is the bottleneck.
- **Building from source — feature flag cheat-sheet** — table of features (`lmdb`, `rocks`, `compiler`, `vectors`, `server`, `dev-instance`, `production`) with when each is needed
- **Common failure modes**
  - Port conflict: change `SPARROW_PORT` or kill process on 6969
  - Docker not running: `sparrow push` requires Docker or Podman daemon
  - Data directory: `SPARROW_DATA_DIR` sets the storage root; ensure the process has write permission
  - Schema compile error: run `sparrow check` before `sparrow push` to get ariadne diagnostics without deploying
- **Exit**: once `/diagnostics` returns 200 → proceed to `querying.md`; if errors → `debugging.md`

---

### 3. `migration.md` — Type A (workflow)

**Purpose**: Schema migrations and data import/export. Scoped to what SparrowDB actually supports (snapshot/restore + structured import); does not cover general "migrate away to another DB" workflows.

**Steps:**

```
Step 1 — Classify the change
  ├── Schema change (new field, type change, rename, new type) → schema migration
  └── Data ingest (CSV/JSON/Parquet from external source)      → bulk import

--- Schema migration path ---

Step 2 — Snapshot first (always)
  sparrow data snapshot
  → hot-copies live DB to directory; safe to run against live instance

Step 3 — Write migration block in .hx file
  schema::1 { ... old schema ... }

  MIGRATION schema::1 => schema::2 {
    Node OldType => NewType {
      old_field => new_field          // rename
      count: I32 => count: I64        // type cast
      status => status = "active"     // literal default
      created_at => created_at = NOW  // timestamp default
    }
  }

  schema::2 { ... new schema ... }

Step 4 — Validate without deploying
  sparrow check
  → ariadne compiler errors point to file:line:col

Step 5 — Deploy
  sparrow push dev    ← test environment first
  sparrow push prod   ← promote after validation

Step 6 — Verify
  GET /introspect     → new schema reflected
  Spot-check a node to confirm field values migrated correctly

--- Bulk import path ---

Step 2 — Prepare data file
  Supported formats: JSON, CSV, Parquet
  Each row/object must map to parameters of an existing QUERY

Step 3 — Import
  sparrow import users.csv --query CreateUser
  sparrow import products.json --query CreateProduct
  sparrow import events.parquet --query ImportEvent

  Flags: --workers N (default 8), --batch-size N, --dry-run, --token <api-key>

Step 4 — Verify
  GET /diagnostics    → node/edge/vector counts increased
  Spot-check via /node_details or a QUERY
```

**Sections after steps:**

- **Migration field transform reference** — table of supported transforms: rename, type cast, literal default, `NOW`, identity (no change)
- **Snapshot & restore commands** — `sparrow data snapshot`, `sparrow data clone`, `sparrow data restore [--force]`
- **LMDB single-writer invariant** — migrations run as write transactions; never run two migrations concurrently. The `WorkerPool` enforces this in Rust but do not attempt to script parallel deployments
- **Common failure modes**
  - Vector dimension mismatch after migration: if `vector(N)` dimension changes, existing vectors become invalid. Drop and re-embed.
  - `UNIQUE INDEX` conflicts during import: use `UpsertN` queries instead of `AddN` to handle duplicates gracefully
  - Migration block order matters: schema version numbers must be contiguous; gaps cause compile errors
  - `sparrow check` fails silently on missing features: run with `--features lmdb,server` when checking from source
- **Exit**: success → `querying.md`; failure → `debugging.md`

---

### 4. `debugging.md` — Type A (workflow)

**Purpose**: Systematic diagnosis and resolution of SparrowDB issues.

**Steps:**

```
Step 1 — Classify the symptom
  ├── A: Compile / build error (ariadne output, cargo error)
  ├── B: Runtime error (HTTP error response with code field)
  ├── C: Wrong results (query returns unexpected data)
  ├── D: Performance (slow queries, high latency, timeouts)
  └── E: Async hang / deadlock (process stops responding)

Step 2 — Run baseline checks
  GET /diagnostics   → node/edge/vector counts, HNSW health
  GET /introspect    → schema matches what you expect

Step 3 — Isolate with runtime eval
  Set SPARROW_RUNTIME_EVAL=1 (env var) to enable:
  POST /__hql_runtime_eval   body: { "query": "N<User>('id') RETURN _" }
  Reproduces the issue without a compiled query endpoint

Step 4 — Branch on symptom (see decision tree below)

Step 5 — Fix, re-check, re-deploy
  sparrow check     ← validate before deploying
  sparrow push dev  ← deploy to test instance
  Repeat Steps 2–3 to confirm resolution
```

**Decision tree by symptom:**

```
A — Compile error
  • Read ariadne output: file:line:col with underline and note
  • Check feature flags: tests need --features lmdb,server
  • Missing ariadne crate? Must be included when `compiler` feature is active
  • Grammar error? Check grammar.pest for PEG rule that matches the failing token

B — Runtime HTTP error
  • INVALID_API_KEY     → check x-api-key header; see docs/auth.md
  • FORBIDDEN           → token role insufficient (need read_write or admin)
  • NOT_FOUND (query)   → query name mismatch; check /introspect for registered routes
  • NOT_FOUND (v1)      → /v1/query route not registered before wildcard /{*path}
  • GRAPH_ERROR         → write_txn() called outside WorkerPool writer thread?
  • VECTOR_ERROR        → embedding dimension mismatch vs vector(N) in schema

C — Wrong results
  • Stale vector results → HNSW soft-delete accumulation (DROP doesn't compact index)
  • Missing nodes        → check WHERE predicate logic; AND vs OR precedence
  • Wrong shape          → inspect field remapping (!{fields}, spread .., closure |var|{})
  • Edge direction wrong → Out<E> vs In<E>; FromN vs ToN

D — Performance
  • Run: sparrow stress <instance>   ← load test
  • Check HNSW health in /diagnostics (entry_point_present, soft_deleted count)
  • High soft_deleted count → index degraded; plan a re-index / re-embed
  • Single-writer contention → all writes serialise through WorkerPool; batch writes where possible
  • Consider RocksDB backend if write throughput is the bottleneck

E — Async hang / deadlock
  • std::process::Command used inside an async fn? → replace with tokio::process::Command
  • write_txn() held across an await point? → LMDB write locks must not cross await boundaries
  • Check for blocked Tokio thread pool with RUST_LOG=tokio=trace
```

**Sections after decision tree:**

- **Enabling debug output** — build with `debug-output` feature flag for verbose macro expansion diagnostics; use `RUST_LOG=sparrow_db=debug` for runtime logging
- **Log streaming** — `sparrow logs <instance>` streams Docker logs from a running instance
- **Dev-only endpoints** — available when `dev-instance` feature is enabled:
  - `POST /node_details` — fetch a node by ID
  - `POST /nodes_by_label` — list all nodes of a type
  - `POST /node_connections` — get edges and neighbours
- **Known HNSW caveats** — soft-delete marks deleted nodes' vectors as inactive in the HNSW graph but does not remove their edges. Over time, stale entries degrade recall precision. Hard delete / compaction is not yet implemented. Mitigation: re-embed the collection into a fresh vector type.
- **Serial test requirement** — LMDB stress tests use `serial_test` to avoid write conflicts: `cargo test --package sparrow-core --features lmdb -- --test-threads=1`

---

## Non-goals

- These files do not duplicate content from `docs/HQL.md`, `docs/HTTP_API.md`, or `docs/auth.md` — they link to those documents
- No "migrate away from SparrowDB to another database" guide (not a documented workflow)
- No SDK documentation (SDKs have their own READMEs in `sdks/rust/` and `sdks/ts/`)

---

## Implementation notes

- Files must be written using information from the live code graph and existing docs — not synthesised from the spec alone
- The code graph (re-indexed 2026-05-23: 358 files, 5,720 nodes, 71,363 edges) is the source of truth for operator names, endpoint paths, feature flags, and error codes
- Each file should be self-contained enough to be useful without reading the others
- Cross-references between skills use relative paths (`../skills/debugging.md`)
