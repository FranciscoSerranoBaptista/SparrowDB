# SparrowDB Monorepo Reorganisation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure the flat-root monorepo into `crates/` + `sdks/` + `tests/`, rename `sparrow-db` → `sparrow-core`, consolidate the duplicate SDK, extract sparrow-cli's inline tests to a proper integration-test directory, and document every directory with README.md and CLAUDE.md.

**Architecture:** All internal Rust crates move into `crates/`, public client SDKs into `sdks/`, standalone test harnesses into `tests/`. The `sparrow-db` package is renamed `sparrow-core` but retains `[lib] name = "sparrow_db"` so all 24+ `use sparrow_db::` import sites in dependent crates need zero changes. The duplicate `sparrow-sdk/` root crate is absorbed into `sdks/rust/`.

**Tech Stack:** Rust workspace (Cargo), git mv (history-preserving), cargo check/test for verification.

---

## Dependency map (read before touching anything)

```
sparrow-macros        ← no internal deps
metrics/              ← no internal deps   (package name already: sparrow-metrics)
hql-tests             ← no internal deps   (talks to a running instance via HTTP/process)
sparrow-sdk           ← no internal deps   (uses helix-dsl-macros from crates.io)
sparrow-ts            ← no internal deps

sparrow-db            ← sparrow-macros, metrics
sparrow-memory        ← sparrow-db
sparrow-container     ← sparrow-db, sparrow-macros
sparrow-cli           ← sparrow-db, metrics
sparrowdb-chef        ← no internal deps
```

---

## Target layout

```
crates/
  sparrow-core/        was sparrow-db/   package: sparrow-core   lib: sparrow_db
  sparrow-cli/         was sparrow-cli/  (moved only)
  sparrow-chef/        was sparrowdb-chef/ package+bin: sparrow-chef
  sparrow-macros/      was sparrow-macros/ (moved only)
  sparrow-memory/      was sparrow-memory/ (moved only)
  sparrow-container/   was sparrow-container/ (moved only)
  sparrow-metrics/     was metrics/      (moved + dir renamed)

sdks/
  rust/                was sparrow-sdk/  package: sparrow-sdk  (sdks/rust already exists as stub)
  ts/                  was sparrow-ts/   (moved only)

tests/
  hql-tests/           was hql-tests/    (moved only)

docs/                  unchanged
examples/              unchanged
scripts/               unchanged
assets/                unchanged
```

---

## File map

| From | To | Change type |
|---|---|---|
| `sparrow-macros/` | `crates/sparrow-macros/` | move |
| `metrics/` | `crates/sparrow-metrics/` | move + dir rename |
| `sparrow-db/` | `crates/sparrow-core/` | move + package rename |
| `sparrow-cli/` | `crates/sparrow-cli/` | move |
| `sparrow-container/` | `crates/sparrow-container/` | move |
| `sparrow-memory/` | `crates/sparrow-memory/` | move |
| `sparrowdb-chef/` | `crates/sparrow-chef/` | move + package rename |
| `sparrow-sdk/` | `sdks/rust/` | merge into existing stub |
| `sparrow-ts/` | `sdks/ts/` | move |
| `hql-tests/` | `tests/hql-tests/` | move |
| `Cargo.toml` | `Cargo.toml` | workspace members update |
| `crates/sparrow-core/Cargo.toml` | same | name + lib section + path fixes |
| `crates/sparrow-cli/Cargo.toml` | same | dep names + paths |
| `crates/sparrow-container/Cargo.toml` | same | dep names + paths |
| `crates/sparrow-memory/Cargo.toml` | same | dep name + path |
| `crates/sparrow-chef/Cargo.toml` | same | package + bin + lib name |
| `sdks/rust/Cargo.toml` | same | package name |
| `crates/sparrow-cli/src/tests/` → `crates/sparrow-cli/tests/` | Task 5 | extract |

---

## Task 1: Move all crates into the new structure (atomic, single commit)

**Files:** Everything listed in the file map above.

> Tasks 1 and 2 are done together — moves first, Cargo.toml updates second, single `cargo check` to validate, single commit. Do not commit Task 1 alone; the workspace will not compile mid-move.

- [ ] **Step 1: Create the three new top-level directories**

```bash
mkdir -p crates sdks tests
```

- [ ] **Step 2: Move no-dependency crates first**

Use `git mv` to preserve history:

```bash
git mv sparrow-macros crates/sparrow-macros
git mv metrics crates/sparrow-metrics
git mv sparrow-ts sdks/ts
```

- [ ] **Step 3: Move the core engine (rename during move)**

```bash
git mv sparrow-db crates/sparrow-core
```

- [ ] **Step 4: Move the remaining internal-dep crates**

```bash
git mv sparrow-cli crates/sparrow-cli
git mv sparrow-container crates/sparrow-container
git mv sparrow-memory crates/sparrow-memory
git mv sparrowdb-chef crates/sparrow-chef
git mv hql-tests tests/hql-tests
```

- [ ] **Step 5: Consolidate the SDK — copy canonical source into sdks/rust, delete root copy**

`sparrow-sdk/src/dsl.rs` is the authoritative file (5370 lines). `sdks/rust/src/` was a stub committed earlier. Replace the stub with the full source:

```bash
cp sparrow-sdk/src/dsl.rs sdks/rust/src/dsl.rs
cp sparrow-sdk/src/lib.rs sdks/rust/src/lib.rs
cp sparrow-sdk/src/query_generator.rs sdks/rust/src/query_generator.rs
git rm -r sparrow-sdk
git add sdks/rust/src/
```

Now proceed to Task 2 before committing.

---

## Task 2: Update all Cargo.toml files (do immediately after Task 1, same commit)

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/sparrow-core/Cargo.toml`
- Modify: `crates/sparrow-cli/Cargo.toml`
- Modify: `crates/sparrow-container/Cargo.toml`
- Modify: `crates/sparrow-memory/Cargo.toml`
- Modify: `crates/sparrow-chef/Cargo.toml`
- Modify: `sdks/rust/Cargo.toml`

- [ ] **Step 1: Update workspace root `Cargo.toml`**

Replace the `members` array entirely:

```toml
[workspace]
members = [
    "crates/sparrow-core",
    "crates/sparrow-container",
    "crates/sparrow-macros",
    "crates/sparrow-cli",
    "crates/sparrow-metrics",
    "crates/sparrow-memory",
    "crates/sparrow-chef",
    "sdks/rust",
    "tests/hql-tests",
]
resolver = "2"
```

- [ ] **Step 2: Update `crates/sparrow-core/Cargo.toml`**

Three changes: rename package, add `[lib]` section to preserve the Rust crate name, fix metrics path.

Change `name`:
```toml
[package]
name = "sparrow-core"
version = "3.0.0"
```

Add a `[lib]` section (insert anywhere after `[package]`):
```toml
[lib]
name = "sparrow_db"
path = "src/lib.rs"
```

This is critical — without it, the default lib name becomes `sparrow_core`, breaking all `use sparrow_db::` in sparrow-cli, sparrow-container, and sparrow-memory without touching a single source file.

Fix the metrics path (it was `../metrics`, now the dir is `../sparrow-metrics`):
```toml
sparrow-metrics = { path = "../sparrow-metrics" }
```

The `sparrow-macros` path stays `../sparrow-macros` — both are siblings inside `crates/`. ✅

- [ ] **Step 3: Update `crates/sparrow-cli/Cargo.toml`**

Replace the sparrow-db dependency key and features; fix the metrics path:

```toml
sparrow-core = { path = "../sparrow-core" }
sparrow-metrics = { path = "../sparrow-metrics" }
```

Update the `[features]` section (the old dep key `sparrow-db` must change in feature forwarding):
```toml
[features]
normal = ["sparrow-core/server"]
ingestion = ["sparrow-core/full"]
default = ["normal"]
```

- [ ] **Step 4: Update `crates/sparrow-container/Cargo.toml`**

```toml
sparrow-core = { path = "../sparrow-core", default-features = false }
sparrow-macros = { path = "../sparrow-macros" }
```

Update features:
```toml
[features]
default = ["lmdb"]
lmdb = ["sparrow-core/lmdb"]
dev = ["sparrow-core/dev-instance"]
production = ["sparrow-core/production"]
```

- [ ] **Step 5: Update `crates/sparrow-memory/Cargo.toml`**

```toml
sparrow-core = { path = "../sparrow-core", default-features = false, features = ["lmdb", "vectors"] }
```

- [ ] **Step 6: Update `crates/sparrow-chef/Cargo.toml`**

Three renames — package name, binary name, lib name:

```toml
[package]
name = "sparrow-chef"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "sparrow-chef"
path = "src/main.rs"

[lib]
name = "sparrow_chef"
path = "src/lib.rs"
```

- [ ] **Step 7: Update `sdks/rust/Cargo.toml`**

Change the package name from `helix-db` to `sparrow-sdk`:

```toml
[package]
name = "sparrow-sdk"
```

Leave `helix-dsl-macros` and all other deps unchanged — those are external crates.io deps.

- [ ] **Step 8: Verify the workspace compiles**

```bash
cargo check --workspace 2>&1 | head -40
```

Expected: zero errors. There may be warnings about unused items — those are fine.

If you see `error[E0433]: failed to resolve: use of undeclared crate or module 'sparrow_db'`, you missed the `[lib] name = "sparrow_db"` addition in Step 2.

If you see `error: no matching package named 'sparrow-db'`, a Cargo.toml still has the old dep key — grep for it: `grep -r "sparrow-db" crates/ sdks/ tests/`

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: restructure monorepo into crates/ sdks/ tests/

Move all internal Rust crates into crates/, client SDK into sdks/rust/,
HQL test harness into tests/hql-tests/. Rename sparrow-db -> sparrow-core
(keeping lib name sparrow_db for import compatibility), sparrowdb-chef ->
sparrow-chef, metrics/ dir -> sparrow-metrics/. Consolidate sparrow-sdk
root crate into sdks/rust/ and remove the duplicate."
```

---

## Task 3: Full test run — verify nothing broke

- [ ] **Step 1: Run unit tests for all workspace crates**

```bash
cargo test --workspace 2>&1 | tail -40
```

Expected: all previously-passing tests pass. The test count should match what existed before the move (19 for sparrow-chef, etc.).

- [ ] **Step 2: Run cargo check with all features**

```bash
cargo check --workspace --all-features 2>&1 | grep "^error" | head -20
```

Expected: no errors. Feature-gated code must compile.

- [ ] **Step 3: Verify the sparrow-chef binary name changed**

```bash
cargo run --package sparrow-chef -- --help 2>&1 | head -5
```

Expected output begins with:
```
Bootstrap a SparrowDB application for a coding agent
```

- [ ] **Step 4: Commit any fixes**

If errors required fixes:
```bash
git add -A
git commit -m "fix: resolve post-reorganisation compile errors"
```

---

## Task 4: Extract sparrow-cli inline tests to integration test directory

The 11 test files in `crates/sparrow-cli/src/tests/` are currently compiled as part of the library crate (they use `crate::` references). Move them to Cargo's standard integration test location at `crates/sparrow-cli/tests/` — each file becomes a separate integration test binary that imports `sparrow_cli::` publicly.

**Files:**
- Move: `crates/sparrow-cli/src/tests/*.rs` → `crates/sparrow-cli/tests/*.rs`
- Modify: `crates/sparrow-cli/src/lib.rs` (remove `mod tests;`)
- Modify: each moved test file (fix `crate::` → `sparrow_cli::`)

- [ ] **Step 1: Inventory crate:: usages in each test file**

```bash
grep -rn "^use crate::\|crate::" crates/sparrow-cli/src/tests/ | grep -v "test_utils" | head -30
```

Note every `crate::X` reference. Each one needs a public API to be accessible from integration tests.

- [ ] **Step 2: Create the integration test directory and move files**

```bash
mkdir -p crates/sparrow-cli/tests
cp crates/sparrow-cli/src/tests/*.rs crates/sparrow-cli/tests/
```

Do not delete the originals yet — keep both until tests pass.

- [ ] **Step 3: Replace `crate::` with `sparrow_cli::` in each moved file**

```bash
sed -i '' 's/use crate::/use sparrow_cli::/g' crates/sparrow-cli/tests/*.rs
sed -i '' 's/crate::/sparrow_cli::/g' crates/sparrow-cli/tests/*.rs
```

- [ ] **Step 4: Run integration tests to see which still need work**

```bash
cargo test --package sparrow-cli 2>&1 | grep "^error" | head -30
```

For each error: if `sparrow_cli::X` is private, add `pub` to the item in `crates/sparrow-cli/src/`. Integration tests can only access public API.

- [ ] **Step 5: Once integration tests pass, remove the old inline tests**

```bash
git rm crates/sparrow-cli/src/tests/*.rs
git rm crates/sparrow-cli/src/tests/mod.rs
```

Remove the `mod tests;` line from `crates/sparrow-cli/src/lib.rs`.

- [ ] **Step 6: Verify**

```bash
cargo test --package sparrow-cli 2>&1 | tail -10
```

Expected: same number of tests as before, all passing.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(cli): move inline tests to integration test directory"
```

---

## Task 5: Write README.md for every directory

One README per location. Keep each under 80 lines. Content: what it does, how to build/run it, key files.

**Files to create:**
- `README.md` (root — already exists, update it)
- `crates/sparrow-core/README.md`
- `crates/sparrow-cli/README.md` (already exists in original sparrow-cli, update path refs)
- `crates/sparrow-chef/README.md`
- `crates/sparrow-macros/README.md`
- `crates/sparrow-memory/README.md`
- `crates/sparrow-container/README.md`
- `crates/sparrow-metrics/README.md`
- `sdks/rust/README.md` (already exists, update package name refs)
- `sdks/ts/README.md`
- `tests/hql-tests/README.md` (already exists in original hql-tests)
- `docs/README.md`

- [ ] **Step 1: Update root `README.md`**

Replace or rewrite to reflect the new structure:

```markdown
# SparrowDB

Open-source graph-vector database built in Rust. Combines graph traversal, 
vector similarity search, and BM25 keyword search in a single embeddable store.

## Repository layout

| Directory | Contents |
|---|---|
| `crates/sparrow-core` | Core database engine — storage, traversal, HTTP gateway, HQL compiler |
| `crates/sparrow-cli` | `sparrow` CLI — init, push, run, check, metrics |
| `crates/sparrow-chef` | `sparrow-chef` — one-command project bootstrap for coding agents |
| `crates/sparrow-macros` | Proc-derive macros for SparrowDB type generation |
| `crates/sparrow-memory` | Episodic memory layer for AI agents, backed by SparrowDB |
| `crates/sparrow-container` | Docker container runtime wrapper |
| `crates/sparrow-metrics` | Optional anonymous telemetry |
| `sdks/rust` | Published Rust client SDK (`sparrow-sdk` on crates.io) |
| `sdks/ts` | TypeScript type definitions and tooling |
| `tests/hql-tests` | End-to-end HQL query language test harness |
| `docs/` | Architecture docs, API specs, migration guides |
| `examples/` | Sample applications (Python, TypeScript) |

## Quick start

```bash
cargo install sparrow-cli
sparrow init my-project && cd my-project
sparrow run
```

## Build

```bash
cargo build --workspace
cargo test --workspace
```

## License

Core engine: AGPL-3.0. Client SDKs: Apache-2.0.
```

- [ ] **Step 2: Write `crates/sparrow-core/README.md`**

```markdown
# sparrow-core

The SparrowDB core engine. Provides graph-vector storage, HQL compilation,
and the HTTP API gateway that all other components talk to.

## Key capabilities

- Dual storage backends: LMDB (default) and RocksDB
- Graph traversal with the HQL query language
- HNSW approximate vector search with soft/hard delete
- BM25 full-text keyword search
- Axum-based HTTP gateway with MCP server support
- Runtime HQL evaluation via `POST /__hql_runtime_eval`

## Build

```bash
# Default (LMDB + server features):
cargo build --package sparrow-core

# All features:
cargo build --package sparrow-core --all-features
```

## Feature flags

| Flag | What it enables |
|---|---|
| `lmdb` | LMDB storage backend (default) |
| `server` | HTTP gateway, HQL compiler, vector search, embeddings |
| `dev-instance` | Debug visualization endpoints (`/nodes-edges` etc.) |
| `production` | API key enforcement |
| `api-key` | API key middleware only |
| `vectors` | HNSW vector index |
| `compiler` | HQL parser + compiler |

## Tests

```bash
cargo test --package sparrow-core
```

## Key source directories

| Path | Contents |
|---|---|
| `src/sparrow_engine/` | Graph/vector traversal operations |
| `src/sparrow_gateway/` | HTTP API, routing, MCP server |
| `src/sparrow_gateway/v1_compat/` | HelixDB JSON DSL compatibility shim |
| `src/protocol/` | Request/response types and error handling |
| `src/utils/` | ID generation, label hashing, aggregation |
```

- [ ] **Step 3: Write `crates/sparrow-chef/README.md`**

```markdown
# sparrow-chef

One-command bootstrap for a new SparrowDB project. Writes project files,
starts the database container, waits for health, and seeds example data.

## Usage

```bash
# Interactive (asks what you want to build):
sparrow-chef chef

# Automatic (skips all prompts, uses defaults):
sparrow-chef chef --auto
sparrow-chef cook --auto   # alias
```

## What it creates

```
<project-dir>/
  docker-compose.yml          SparrowDB container config
  db/schema.hx                HQL node/edge type definitions
  db/queries.hx               HQL query stubs
  examples/seed.json          Seed data (POST to /v1/query)
  examples/read.json          Read query example
  SPARROWDB_CHEF_PROMPT.md    Coding-agent prompt with your intent
```

## Build

```bash
cargo build --package sparrow-chef
cargo test --package sparrow-chef   # 19 unit tests
```
```

- [ ] **Step 4: Write remaining READMEs**

`crates/sparrow-macros/README.md`:
```markdown
# sparrow-macros

Proc-derive macros for SparrowDB type generation and introspection.

## Usage

Add to `Cargo.toml`:
```toml
sparrow-macros = { path = "../sparrow-macros" }
```

## Feature flags

| Flag | What it enables |
|---|---|
| `debug-output` | Print generated macro expansions during compilation |

## Build

```bash
cargo build --package sparrow-macros
```
```

`crates/sparrow-memory/README.md`:
```markdown
# sparrow-memory

Lightweight episodic memory system for AI research agents, backed by SparrowDB.
Stores and retrieves agent interactions using the embedded LMDB backend.

## Build

```bash
cargo build --package sparrow-memory
cargo test --package sparrow-memory
```

## Usage

```rust
use sparrow_memory::MemoryStore;
let store = MemoryStore::open("/path/to/data").await?;
store.record(interaction).await?;
let recent = store.recall(query, k).await?;
```
```

`crates/sparrow-container/README.md`:
```markdown
# sparrow-container

Docker container runtime and deployment wrapper for SparrowDB instances.
Handles container lifecycle, environment configuration, and health management.

## Feature flags

| Flag | What it enables |
|---|---|
| `lmdb` | LMDB backend (default) |
| `dev` | Development instance with debug endpoints |
| `production` | Production hardening (API key enforcement) |

## Build

```bash
cargo build --package sparrow-container
```
```

`crates/sparrow-metrics/README.md`:
```markdown
# sparrow-metrics

Optional anonymous telemetry for SparrowDB instances. Sends aggregate usage
events over HTTP. Disabled by default — opt-in via the `SPARROW_METRICS`
environment variable.

## Build

```bash
cargo build --package sparrow-metrics
```
```

`sdks/ts/README.md`:
```markdown
# sparrow-sdk (TypeScript)

TypeScript type definitions and IR representation for SparrowDB schemas.

> **Status:** Early development. Not yet published.

## Structure

- `ir.ts` — Intermediate representation for SparrowDB schema types
- `main.ts` — Entry point

## Build

```bash
deno check main.ts
```
```

`docs/README.md`:
```markdown
# docs

Architecture documentation, API specifications, migration guides, and
implementation plans for SparrowDB.

## Key documents

| File | Contents |
|---|---|
| `HTTP_API.md` | Full HTTP API reference for the SparrowDB gateway |
| `V1_COMPAT_ENDPOINT.md` | HelixDB v1/query compatibility shim documentation |
| `architecture/` | Architecture decision records and system design docs |
| `bugs/` | Known issue tracker |
| `superpowers/plans/` | Claude Code implementation plans (agentic task specs) |
```

- [ ] **Step 5: Update `sdks/rust/README.md`**

The README already exists and is comprehensive. Only update the package name reference from `helix-db` to `sparrow-sdk`:

```bash
sed -i '' 's/helix-db/sparrow-sdk/g; s/helix_db/sparrow_sdk/g' sdks/rust/README.md
```

- [ ] **Step 6: Commit all READMEs**

```bash
git add README.md crates/*/README.md sdks/*/README.md tests/*/README.md docs/README.md
git commit -m "docs: add README.md to every crate and top-level directory"
```

---

## Task 6: Write CLAUDE.md for complex directories

CLAUDE.md captures institutional knowledge for AI coding assistants — the **why** behind non-obvious decisions, known pitfalls, and critical invariants that aren't visible in the code.

Write CLAUDE.md for: root, `crates/sparrow-core`, `crates/sparrow-cli`, `sdks/rust`.

**Files:**
- Create: `CLAUDE.md` (root)
- Create: `crates/sparrow-core/CLAUDE.md`
- Create: `crates/sparrow-cli/CLAUDE.md`
- Create: `sdks/rust/CLAUDE.md`

- [ ] **Step 1: Write root `CLAUDE.md`**

```markdown
# CLAUDE.md — SparrowDB workspace

## Repository structure

All internal Rust crates live in `crates/`. Client SDKs in `sdks/`. 
End-to-end test harnesses in `tests/`. Never add a new crate to the 
workspace root.

## Workspace members

Defined in root `Cargo.toml`. When adding a new crate, add it to `members`
AND create a README.md in its directory.

## crate naming convention

All crate directories and package names follow `sparrow-<role>` (hyphenated).
Binary names match package names. Lib names may differ (see sparrow-core).

## The sparrow-core / sparrow_db naming split

The core engine package is named `sparrow-core` in Cargo.toml but its Rust 
lib is named `sparrow_db` (set via `[lib] name = "sparrow_db"` in 
`crates/sparrow-core/Cargo.toml`). This means:
- Cargo.toml dependencies use: `sparrow-core = { path = "../sparrow-core" }`
- Rust source files use: `use sparrow_db::...`
- NEVER rename the lib without updating all import sites across sparrow-cli, sparrow-container, sparrow-memory.

## SDK licensing

`crates/` are AGPL-3.0. `sdks/` are Apache-2.0. Do not import internal 
crates from `sdks/` — they must stay dependency-free from the core.

## std::process::Command is banned in async code

Any code running inside a Tokio runtime must use `tokio::process::Command`,
not `std::process::Command`. The blocking `.status()` call on std's Command
will stall the async executor. This has caused production issues before.
`cargo clippy` does not catch it — code review must.

## LMDB single-writer invariant

LMDB allows exactly one write transaction at a time. All writes in 
sparrow-core go through a dedicated writer thread. Never acquire a write 
transaction from an async context or a read-path thread.

## Feature flag discipline

`sparrow-core` has many feature flags. The two most important:
- `lmdb` — enables LMDB storage (always required for a running instance)
- `server` — enables the HTTP gateway, compiler, and vector search

Tests that need a running database require `lmdb`. Tests that exercise 
the HTTP API require `server`. Don't run tests without appropriate features.

## Running the full test suite

```bash
cargo test --workspace
```

For sparrow-core with all features:
```bash
cargo test --package sparrow-core --features lmdb,server
```

## Commit conventions

Subject line: `type(scope): description`
Types: feat, fix, refactor, docs, test, chore, build
Scope: crate name (sparrow-core, sparrow-cli, sdks/rust, etc.)
```

- [ ] **Step 2: Write `crates/sparrow-core/CLAUDE.md`**

```markdown
# CLAUDE.md — sparrow-core

## What this crate is

The entire SparrowDB database in one crate. Owns storage, query execution,
vector index, HTTP API, and the HQL compiler. Everything else in this repo
depends on this crate.

## Critical: lib name vs package name

Package name: `sparrow-core`
Rust lib name: `sparrow_db`

This split exists so callers can `use sparrow_db::` without source changes 
after the package rename. The `[lib] name = "sparrow_db"` line in Cargo.toml
MUST NOT be removed or changed without updating all import sites in sparrow-cli,
sparrow-container, and sparrow-memory.

## Directory layout

```
src/
  sparrow_engine/       Core traversal and mutation ops
    traversal_core.rs   SparrowGraphEngine — the main entry point for all queries
    ops/                Individual step implementations (source, filter, mutate)
  sparrow_gateway/      HTTP server layer
    gateway.rs          Axum router setup and request dispatch
    worker_pool.rs      Multi-threaded read pool + single writer thread
    v1_compat/          HelixDB JSON DSL compatibility shim (POST /v1/query)
    builtin/            Built-in endpoints (diagnostics, node_details, etc.)
    mcp/                MCP server for AI agent tool use
  protocol/             Request/response types shared with the gateway
  utils/                ID generation, label hashing, properties
```

## LMDB writer thread

There is exactly one LMDB writer thread. All mutation operations
(`AddNode`, `AddEdge`, `UpdateNode`, `DropNode`) must go through
`WorkerPool::process_write()`, never directly from a read thread.
The gateway routes write-registered query names to the write path automatically.

## v1/query compatibility endpoint

`POST /v1/query` translates HelixDB JSON DSL to SparrowDB ops.
Source: `src/sparrow_gateway/v1_compat/mod.rs`.
This is a migration shim, not a permanent API. See `docs/V1_COMPAT_ENDPOINT.md`.
The route MUST be registered before `/{*path}` in `gateway.rs` — the wildcard
rejects paths containing `/`.

## sonic_rs 0.5.7 API notes

- No `as_object_mut()` method — build objects with `sonic_rs::json!({})` 
- `sonic_rs::Array` derefs to `[Value]` — use `.map(|a| &**a)` not `&[]`
- No `Value::Null` constant — use `Option` or conditional logic
- Use `json!({})` not `object!{}` when the return type must be `Value`

## Vector index (HNSW)

Soft-delete marks vectors as deleted without updating the graph.
Hard-delete removes the data record but leaves graph edges stale.
Always call `rebuild_vector_index` after hard deletes to clean the graph.
The `hnsw_health` and `hnsw_integrity` built-in endpoints verify graph health.

## Adding a new built-in endpoint

1. Create a handler in `src/sparrow_gateway/builtin/<name>.rs`
2. Import it in `gateway.rs` (behind `#[cfg(feature = "dev-instance")]` if dev-only)
3. Register the route in `SparrowGateway::run()`
4. Document in `docs/HTTP_API.md`
```

- [ ] **Step 3: Write `crates/sparrow-cli/CLAUDE.md`**

```markdown
# CLAUDE.md — sparrow-cli

## What this crate is

The `sparrow` CLI binary. Manages the full project lifecycle: scaffold 
a new project, compile and push HQL queries, run a local instance, 
check schemas, send metrics.

## Binary vs library

The crate exposes both a binary (`sparrow`) and a library (`sparrow_cli`).
The library exists to allow integration tests to import internal functions.
Public API in `lib.rs` is intentionally minimal.

## Commands

| Command | File | What it does |
|---|---|---|
| `sparrow init` | `commands/init.rs` | Scaffold a new SparrowDB project |
| `sparrow push` | `commands/push.rs` | Compile HQL and push to running instance |
| `sparrow check` | `commands/check.rs` | Validate schema and queries without pushing |
| `sparrow run` | `commands/build.rs` | Build and start a local Docker instance |
| `sparrow data` | `commands/data.rs` | Snapshot and restore database |
| `sparrow metrics` | `commands/metrics.rs` | Show telemetry data |

## Project discovery

`project.rs` walks parent directories looking for `sparrow.toml`.
All commands require a project root. If not found, most commands error early.

## Docker integration

`docker.rs` uses `tokio::process::Command` (NOT `std::process::Command`).
The `std` version blocks the Tokio runtime — never use it inside async fn.
This has been a recurring source of bugs.

## Integration tests

Tests live in `tests/` (Cargo integration test convention).
They start Docker containers and make real HTTP requests — they are slow.
Run selectively: `cargo test --package sparrow-cli <test_name>`.
The `serial_test` crate is used to serialize tests that share Docker state.

## Feature flags

- `normal` (default): includes `sparrow-core/server` — standard gateway features
- `ingestion`: includes `sparrow-core/full` — adds bulk data import paths

## helix-enterprise-ql dependency

The CLI currently depends on `helix-enterprise-ql` from crates.io.
This is a legacy external crate that will be replaced with HQL-native
equivalents as the HQL compiler matures.
```

- [ ] **Step 4: Write `sdks/rust/CLAUDE.md`**

```markdown
# CLAUDE.md — sdks/rust (sparrow-sdk)

## What this is

The published Rust client SDK for SparrowDB. Crate name on crates.io: `sparrow-sdk`.
Apache-2.0 licensed (permissive, unlike the AGPL-3.0 core).

## Critical: no dependency on internal crates

This SDK must NOT import from `crates/sparrow-core` or any other `crates/` 
package. It is a standalone crate for external users. Its only non-std 
dependencies are `helix-dsl-macros` (external proc macro on crates.io)
and reqwest for HTTP.

## The DSL (dsl.rs)

`dsl.rs` is 5000+ lines. It is generated/maintained as a single file 
intentionally — external users vendor it. Do not split it into modules 
without understanding the downstream impact on crate users who may copy-paste.

The DSL builds query batches that are sent as JSON over HTTP to `/v1/query`
or to compiled query endpoints. It does NOT execute queries locally.

## query_generator.rs

Contains the `#[register]` macro support and bundle generation. This is
how named queries are compiled and pushed to a SparrowDB instance via
`sparrow push`.

## Versioning

The SDK version is independent of sparrow-core. Client-facing breaking 
changes require a semver major bump. Internal core changes that don't affect 
the HTTP API or DSL shape do not require an SDK release.

## Updating the SDK

When the HTTP API adds new endpoints or the v1/query DSL gains new step types,
update `dsl.rs` to expose them. Then run:

```bash
cargo test --package sparrow-sdk
cargo doc --package sparrow-sdk --open
```
```

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md crates/sparrow-core/CLAUDE.md crates/sparrow-cli/CLAUDE.md sdks/rust/CLAUDE.md
git commit -m "docs: add CLAUDE.md institutional knowledge files for AI coding assistants"
```

---

## Self-Review

### Spec coverage

| Requirement | Task |
|---|---|
| Restructure into `crates/` + `sdks/` + `tests/` | Task 1 |
| Rename `sparrow-db` → `sparrow-core` | Task 1-2 |
| `[lib] name` trick to avoid source changes | Task 2, Step 2 |
| Consolidate duplicate SDK | Task 1, Step 5 |
| Rename `sparrowdb-chef` → `sparrow-chef` | Task 2, Step 6 |
| Rename `metrics/` dir → `sparrow-metrics/` | Task 1, Step 2 |
| `cargo check --workspace` passes | Task 2, Step 8 |
| `cargo test --workspace` passes | Task 3 |
| Extract sparrow-cli inline tests | Task 4 |
| README.md for every directory | Task 5 |
| Root CLAUDE.md | Task 6, Step 1 |
| `crates/sparrow-core/CLAUDE.md` | Task 6, Step 2 |
| `crates/sparrow-cli/CLAUDE.md` | Task 6, Step 3 |
| `sdks/rust/CLAUDE.md` | Task 6, Step 4 |

No gaps found.

### Type consistency

- `sparrow-core` package name, `sparrow_db` lib name used consistently in Tasks 2 and 6.
- All feature forwarding (`sparrow-core/server` etc.) matches the package key name, not the lib name.
- `sparrow-chef` binary and package names match throughout Tasks 2 and 5.
