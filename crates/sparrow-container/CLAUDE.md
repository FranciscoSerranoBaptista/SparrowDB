# sparrow-container

Thin deployable binary that wires `sparrow-core` into a Docker-runnable server.

---

## What this crate does / doesn't do

**DOES:**
- Wire `sparrow-core` into a runnable binary (`src/main.rs`)
- Register built-in query handlers and routes (`src/queries.rs`)
- Serve as the Docker deployment target (`sparrow push` / `sparrow run`)
- Read env vars at startup (`SPARROW_DATA_DIR`, `SPARROW_PORT`, `SPARROW_DB_MAX_SIZE_GB`, `SPARROW_SKIP_BM25_ON_WRITE`)

**DOES NOT:**
- Contain business logic — keep it in `sparrow-core`
- Replicate gateway, storage, or compiler code
- Export a library — this is a binary-only crate (`publish = false`)

---

## Dependency note — `default-features = false`

```toml
sparrow-core = { path = "../sparrow-core", default-features = false }
```

This is **intentional**. Feature flags are supplied explicitly at build time (Docker build
args), not defaulted here. The crate's own `[features]` section re-exports them:

| Feature      | Enables                          |
|--------------|----------------------------------|
| `lmdb`       | `sparrow-core/lmdb` (storage)    |
| `studio`     | `sparrow-core/studio`            |
| `dev`        | `sparrow-core/dev-instance` + studio |
| `production` | `sparrow-core/production`        |

When adding features, check `crates/sparrow-core/Cargo.toml` for available feature names.
**Never add `default-features = true`** — it breaks RocksDB builds.

---

## Import rule

The dep is named `sparrow-core` in `Cargo.toml`, but the lib name is `sparrow_db`.
Always import as:

```rust
use sparrow_db::...;
```

See the workspace CLAUDE.md for the full explanation of this naming split.

---

## LMDB single-writer

All mutation paths wired here must go through `WorkerPool::process_write()` in
`sparrow-core`. New endpoints registered via `#[handler(is_write = true)]` are
automatically routed to the single writer. Never open `write_txn()` directly.

---

## Agent invocation guide

| Situation | Agent |
|-----------|-------|
| Any code change | `rust-reviewer` |
| Build or feature flag failure | `rust-build-resolver` |
| Binary latency or memory growth | `sparrow-perf-profiler` |

---

## Skills reference

| Task | Skill |
|------|-------|
| Building and deploying an instance | `docs/skills/setup.md` |
| Debugging runtime issues | `docs/skills/debugging.md` |

---

## Code graph

| Goal | Tool |
|------|------|
| How sparrow-container wires sparrow-core | `get_architecture_overview_tool` |
| Startup sequence | `get_flow_tool` with `main` |
| Impact of touching queries.rs | `get_impact_radius_tool` with `queries` |
| Find where a query handler is registered | `semantic_search_nodes_tool` |
| Quick context on a single file | `get_minimal_context_tool` |
