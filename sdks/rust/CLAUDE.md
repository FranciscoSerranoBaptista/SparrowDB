# sparrow-sdk (sdks/rust) CLAUDE.md

Apache-2.0 public Rust SDK for SparrowDB. Async HTTP client + query builder DSL. Standalone publishable to crates.io.

---

## License boundary — CRITICAL

**This crate is Apache-2.0. The rest of the repo is AGPL-3.0.**

- ZERO internal crate dependencies — never add `path = "../../crates/..."` to `Cargo.toml`
- Adding `sparrow-core`, `sparrow-macros`, or any `crates/` member breaks the license boundary and makes the crate unpublishable to crates.io
- All SparrowDB communication goes through HTTP — never through direct Rust API calls into internal crates
- If you need to share a type with the server, duplicate it here

Current external deps only: `chrono`, `serde`, `sonic-rs`, `inventory`, `helix-dsl-macros`, `reqwest`, `tokio`, `thiserror`.

---

## Key source files

| File | Purpose |
|------|---------|
| `src/lib.rs` | SDK entry point — `Client`, `QueryBuilder`, `QueryRequest`, public API surface |
| `src/dsl.rs` | Query builder DSL — all AST types, builder methods, traversal combinators |
| `src/query_generator.rs` | `#[register]` macro infrastructure — `QueryBundle`, param types, stored query generation |

---

## Edition note

This crate uses **Rust 2021 edition** (not 2024). Keep it at 2021 for crates.io compatibility until 2024 is broadly stable.

---

## Testing

Most tests in `src/lib.rs` are self-contained (no network). Integration tests require a live instance:

```bash
sparrow run                        # start a local SparrowDB instance first
cargo test --package sparrow-sdk   # run all SDK tests
```

---

## Agent invocation guide

| Agent | When to invoke |
|-------|---------------|
| `rust-reviewer` | Any SDK change — will flag internal crate dep violations as a critical finding |
| `silent-failure-hunter` | HTTP client error handling gaps — SDK client code often swallows non-200 errors |
| `rust-build-resolver` | Build failures — workspace-aware diagnosis |

---

## Skills reference

| Skill | When to use |
|-------|------------|
| `docs/skills/querying.md` | HQL query reference — use when extending the DSL to match new query syntax |
| `docs/skills/debugging.md` | SDK-to-server communication issues — request/response tracing |

---

## Code graph

| Tool | When to use |
|------|-------------|
| `get_architecture_overview_tool` | SDK module structure before adding new DSL methods |
| `get_flow_tool` | Trace DSL builder → HTTP request → response deserialization |
| `get_impact_radius_tool` with `query_generator` | See what depends on the query generator before changing it |
| `semantic_search_nodes_tool` | Find a specific SDK method or type by concept |
