# SparrowDB Workspace CLAUDE.md

Institutional knowledge for AI coding assistants. Read this before touching any crate.

---

## Repository structure

```
crates/          <- all first-party Rust crates (AGPL-3.0)
sdks/            <- client SDKs (Apache-2.0)
tests/           <- test harnesses (hql-tests, etc.)
```

Workspace members defined in the root `Cargo.toml`:
- `crates/sparrow-core` — storage engine, HTTP gateway, compiler
- `crates/sparrow-container` — deployable server binary
- `crates/sparrow-macros` — proc-macros used by sparrow-core
- `crates/sparrow-cli` — the `sparrow` CLI binary
- `crates/sparrow-metrics` — metrics collection
- `crates/sparrow-memory` — memory utilities
- `crates/sparrow-chef` — cargo-chef planner for Docker layer caching
- `sdks/rust` — the `sparrow-sdk` crate
- `tests/hql-tests` — integration test harness

**Never add new crates at the workspace root.** All crates go under `crates/` and new SDKs go under `sdks/`.

---

## The sparrow-core / sparrow_db naming split

This is the single most common source of confusion.

- **Package name** (in `Cargo.toml`): `sparrow-core`
- **Library name** (in `[lib] name = "sparrow_db"`): `sparrow_db`

Because the `[lib]` section overrides the default crate name, every crate that depends on `sparrow-core` must import it as:

```rust
use sparrow_db::...;
```

**NEVER remove the `[lib]` section from `crates/sparrow-core/Cargo.toml`** without first updating every `use sparrow_db::` import site across:
- `crates/sparrow-cli`
- `crates/sparrow-container`
- `crates/sparrow-memory`

Search before touching: `grep -r "use sparrow_db" crates/`

---

## std::process::Command is banned in async code

Using `std::process::Command` inside an async function blocks the Tokio thread pool and has caused production hangs.

**Always use `tokio::process::Command` in async contexts.**

```rust
// WRONG — blocks the Tokio runtime
let output = std::process::Command::new("docker").status()?;

// CORRECT
let output = tokio::process::Command::new("docker").status().await?;
```

Note: `crates/sparrow-cli/src/docker.rs` currently uses `std::process::Command` in synchronous helper functions that are only called from blocking contexts. Any refactor that makes those async MUST switch them to `tokio::process::Command`.

---

## LMDB single-writer invariant

LMDB (via `heed3`) enforces a single write transaction at a time at the OS level. The gateway enforces this in Rust too: **all mutations must go through the dedicated writer thread** in `WorkerPool`. There is exactly one `_writer_worker` that holds the `write_rx` channel.

Never open a `write_txn()` outside of the writer thread path. If you add a new mutation endpoint, mark it as a write route so `WorkerPool::process_write()` routes it to the single writer.

---

## Feature flag discipline

```
lmdb    = enables storage backend (heed3). Required for any test that touches the graph.
server  = enables HTTP API (axum routes, compiler, vectors). Required for gateway tests.
```

Tests that exercise the gateway need both:
```
cargo test --features lmdb,server
```

The default feature is `lmdb` which also pulls in `server`. For minimal builds (e.g. testing the compiler in isolation) you can use `--no-default-features --features compiler`.

Key feature chain: `lmdb` → `server` → `build` + `compiler` + `vectors`

---

## SDK licensing

| Location | License | Rule |
|----------|---------|------|
| `crates/` | AGPL-3.0 | May depend on each other |
| `sdks/` | Apache-2.0 | **Zero dependency on internal crates** |

The Rust SDK (`sdks/rust`) must be publishable to crates.io as a standalone crate. It must never import from `sparrow-core`, `sparrow-macros`, or any other `crates/` member.

---

## Running tests

Full workspace test suite:
```bash
cargo test --workspace
```

Tests that need storage:
```bash
cargo test --workspace --features lmdb,server
```

Serialize LMDB stress tests (they use `serial_test` to avoid write conflicts):
```bash
cargo test --package sparrow-core --features lmdb -- --test-threads=1
```

---

## Commit conventions

```
type(scope): description
```

Examples:
```
feat(gateway): add /introspect endpoint
fix(cli): use tokio::process::Command in docker build
refactor(sparrow-core): rename lib to sparrow_db
docs: add CLAUDE.md institutional knowledge files
```

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`

---

## Profiling tools (performance & memory phase)

| Tool | Install | Purpose |
|------|---------|---------|
| `cargo flamegraph` | `cargo install flamegraph` | CPU flame graph — identify hot functions |
| `cargo +nightly dhat` | nightly toolchain | Heap allocation profile by call site |
| `heaptrack` | system package (Linux) | Live heap growth over time |
| `criterion` | dev-dependency in crate | Reproducible microbenchmarks |
| `sparrow stress` | built-in CLI | End-to-end load test against a live instance |
| `SPARROW_SKIP_BM25_ON_WRITE=1` | env var | Isolate BM25 rebuild cost from write latency |
| `POST /rebuild_bm25_index` | HTTP endpoint | Trigger and time a manual BM25 index rebuild |

Use the `sparrow-perf-profiler` agent for a structured four-phase workflow combining these tools.

---

## Agent invocation guide

Agents live in `.agents/`. Invoke via the Claude Code `Agent` tool or by spawning a sub-agent
with `subagent_type` set to the agent name.

| Agent | When to invoke |
|-------|---------------|
| `rust-reviewer` | Before merging any Rust change — runs clippy + safety + SparrowDB invariant checks |
| `rust-build-resolver` | When `cargo build` / `cargo check` fails — workspace-aware diagnosis |
| `silent-failure-hunter` | When a write or query path produces wrong results silently — error propagation audit |
| `sparrow-perf-profiler` | When latency or memory grows unexpectedly — four-phase profiling workflow |

---

## Code graph

The codebase is indexed in the `code-review-graph` MCP server. Use these tools for deep
structural understanding before making changes:

| Tool | When to use |
|------|-------------|
| `build_or_update_graph_tool` | After adding new files or major refactors — refresh the index |
| `get_architecture_overview_tool` | Before starting a new feature — understand the full structure |
| `semantic_search_nodes_tool` | Find where a concept is implemented (e.g. "write_txn", "HNSW insert") |
| `get_review_context_tool` | Before reviewing a PR — get context on every changed file |
| `get_impact_radius_tool` | Before modifying a function — see what depends on it |
| `get_minimal_context_tool` | Quick context on a single file without reading the whole codebase |
| `get_flow_tool` | Trace a code path end-to-end (e.g. HTTP request → storage write) |
| `traverse_graph_tool` | Explore dependencies and callers of a specific node |

Rebuild the graph after significant structural changes:
```bash
# via MCP tool in Claude Code session
build_or_update_graph_tool(repo_path="/Users/franciscobaptista/Development/SparrowDB")
```
