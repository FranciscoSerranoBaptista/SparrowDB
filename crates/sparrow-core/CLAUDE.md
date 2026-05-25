# sparrow-core CLAUDE.md

The heart of SparrowDB — storage engine, HTTP gateway, and HQL compiler in one crate.

---

## Key source directories

| Path | Contents |
|------|----------|
| `src/sparrow_engine/` | Storage core: LMDB backend (heed3), BM25 index, HNSW vector index, graph traversal, reranker |
| `src/sparrow_gateway/` | HTTP gateway (axum), WorkerPool (single-writer), auth, MCP server, embedding providers |
| `src/sparrowc/` | HQL compiler: parser → analyzer → generator |
| `src/protocol/` | Shared data types, error types (`GraphError`, `VectorError`) |
| `src/grammar.pest` | PEG grammar for HQL — source of truth for the parser; edit here first |

---

## Agent invocation guide

Dispatch as a sub-agent via the Agent tool. Agents live in `.agents/<name>.md` — read the frontmatter for model and tool requirements.

| Situation | Agent | Why |
|-----------|-------|-----|
| Reviewing any change | `rust-reviewer` | Checks SparrowDB invariants (LMDB, async, error propagation) before generic style |
| Error handling gaps / swallowed errors | `silent-failure-hunter` | Finds dangerous fallbacks and missing propagation paths |
| Slow queries / high memory / HNSW degradation | `sparrow-perf-profiler` | Four-phase LMDB/HNSW/BM25/Tokio profiling workflow |
| Build failure / feature flag confusion | `rust-build-resolver` | Knows the lib-name split and feature chain |

---

## Skills reference

- Debugging a runtime issue → `docs/skills/debugging.md`
- Writing or reviewing HQL → `docs/skills/querying.md`

---

## Local invariants

1. **Package/library name split.** The package name is `sparrow-core` but the library name is `sparrow_db` (set via `[lib] name = "sparrow_db"` in `Cargo.toml`). Every consumer imports as `use sparrow_db::...`. Never remove the `[lib]` section without first updating all import sites — search with `grep -r "use sparrow_db" crates/`.

2. **LMDB single-writer.** LMDB enforces one write transaction at the OS level; the gateway mirrors this in Rust via `WorkerPool`. All mutations must flow through the dedicated `_writer_worker` that owns `write_rx`. Never call `write_txn()` outside that thread path. New mutation endpoints must be registered as write routes so `WorkerPool::process_write()` routes them correctly.

3. **`std::process::Command` is banned in async.** Blocking the Tokio thread pool causes production hangs. Always use `tokio::process::Command` inside any `async fn`. The only current exception is `sparrow-cli/src/docker.rs`, which runs in a synchronous context — any async refactor there must switch it over.

4. **Feature flag chain.** `lmdb` → `server` → `build + compiler + vectors`. Tests that touch the graph need `--features lmdb,server`. Compiler-only builds: `--no-default-features --features compiler`. The `ariadne` dependency is required whenever the `compiler` feature is active — do not remove it.

5. **GraphError propagation.** Always propagate errors using the specific `GraphError` variant that matches the failure. Never map to `GraphError::Unknown` — that loses the structured context that callers and log consumers rely on. The `silent-failure-hunter` agent can audit for violations.

---

## HNSW caveats

- `DROP` on a node marks its vector **soft-deleted** in the HNSW graph but does not compact the index.
- Stale soft-deleted entries accumulate over time and degrade approximate-nearest-neighbour recall.
- There is no hard-delete or compaction path yet — this is a known limitation.
- Current mitigation: re-embed affected data into a fresh vector type to get a clean index.
- `GET /diagnostics` reports `soft_deleted` count per vector type — monitor this in long-running instances.

---

## Code graph

Use the `code-review-graph` MCP tools to navigate sparrow-core without reading entire files.

**Start here:**
- Architecture overview: `get_architecture_overview_tool` — maps modules, their sizes, and dependency clusters
- Find a function: `semantic_search_nodes_tool` — search by meaning, e.g. "write transaction LMDB"

**Common queries for this crate:**
- Trace the write path: `get_flow_tool` with entry point `WorkerPool::process_write`
- Impact of changing `write_txn`: `get_impact_radius_tool` with target `write_txn`
- HNSW insert flow: `get_flow_tool` with entry point `vector_core::insert`
- High-connectivity nodes (change these carefully): `get_hub_nodes_tool`
- Before touching `GraphError`: `get_impact_radius_tool` with target `GraphError`
- Minimal context for a symbol: `get_minimal_context_tool` with the function/type name

**Before any PR touching sparrow_engine or sparrow_gateway:**
Run `get_impact_radius_tool` on the changed symbol to understand blast radius.

---

## Additional notes

**v1/query compatibility.** `src/sparrow_gateway/v1_compat/mod.rs` provides `POST /v1/query` as a bridge for HelixDB callers. This route **must be registered before `/{*path}`** in `gateway.rs` — the wildcard rejects paths containing `/`. The endpoint peeks at raw request bytes for the string `"write"` to route to the correct worker channel, preserving the single-writer invariant.

**sonic_rs 0.5.7.** The codebase uses `sonic-rs = "0.5.7"`. No `as_object_mut()` — mutate by reconstruction. Import `JsonContainerTrait` and `JsonValueTrait` for `.get()`, `.as_str()`, `.as_i64()` to work on `Value`. Use `sonic_rs::json!{}`, not `object!{}`.

**sparrow-macros.** This crate is `proc-macro = true` — it cannot be used as a normal library dependency. Import it only as a proc-macro in `[dependencies]` with `proc-macro = true` semantics.
