# sparrow-sdk (sdks/rust) CLAUDE.md

The official Rust client SDK for SparrowDB. Read this before changing the DSL, the query generator, or the HTTP client.

---

## No dependency on internal crates

This crate is **Apache-2.0** and must be publishable to crates.io as a standalone package. It must never import from `sparrow-core`, `sparrow-macros`, `sparrow-container`, or any other crate under `crates/`.

Current external dependencies only: `chrono`, `serde`, `sonic-rs`, `inventory`, `helix-dsl-macros`, `reqwest`, `tokio`, `thiserror`.

If you need to share a type with the server, duplicate it. Do not create a shared crate in `crates/` that this SDK depends on — that would make the SDK AGPL by proxy.

Note: `src/lib.rs` contains two `extern crate self as` aliases for backward compatibility:
```rust
extern crate self as sparrow_sdk;
extern crate self as helix_db;  // kept while helix-dsl-macros still emits ::helix_db:: paths
```
The `helix_db` alias can be removed once `helix-dsl-macros` is updated to emit `::sparrow_sdk::` paths.

---

## The DSL (dsl.rs)

`src/dsl.rs` is a single large file (~5000+ lines). It is intentionally kept as one file because external users who vendor the SDK copy it as a single unit. Splitting it into submodules would break vendored copies.

The file contains:
- All query AST types (`ReadBatch`, `WriteBatch`, `NodeStep`, `EdgeStep`, `SourcePredicate`, `Predicate`, etc.)
- Builder APIs (`read_batch()`, `write_batch()`, `g()`)
- Traversal combinators (`.n()`, `.out()`, `.in_()`, `.where_()`, `.limit()`, `.order_by()`, `.value_map()`, etc.)
- Write operations (`.add_n()`, `.add_e()`, `.update()`, `.drop()`)
- Conditional query support (`BatchCondition`, `.var_as_if()`)
- Parameter expression support (`Expr::param()`, `DynamicQueryValue`, `DynamicQueryRequest`)
- Serde implementations for the wire format

The public entry point for application code is the `prelude` module:
```rust
use sparrow_sdk::dsl::prelude::*;
```

---

## query_generator.rs

`src/query_generator.rs` provides the infrastructure for the `#[register]` proc-macro (from `helix-dsl-macros`). The macro transforms a function that builds a query DSL expression into a registered route entry.

Key types:
- `QueryBundle` — the versioned JSON file written to `queries.json` that the server loads. Current wire format version: `QUERY_BUNDLE_VERSION = 4`.
- `RegisteredReadQuery` / `RegisteredWriteQuery` — inventory entries collected at startup.
- `QueryParamType` / `QueryParameter` — parameter shape metadata emitted alongside each route.

The `#[register]` macro on a function:
```rust
#[register]
fn get_user(name: String) {
    read_batch()
        .var_as("user", g().n_where(SourcePredicate::eq("username", name)))
        .returning(["user"])
}
```

Generates:
1. A callable function `get_user(name: String) -> DynamicQueryRequest` for use with the HTTP client.
2. An `inventory::submit!` registration that adds the route to `QueryBundle` at bundle-generation time.

To generate `queries.json` from a project that uses `#[register]`, run `cargo run` in the queries project directory. This is what `sparrow compile` and `sparrow push` do internally via `run_enterprise_compile()`.

---

## Versioning

The SDK version (`sparrow-sdk = "1.0.0"`) is **independent of the server version** (`sparrow-core = "3.0.0"`). The SDK communicates with the server over HTTP, so minor wire-format differences are handled by the server's compatibility layer.

The `QueryBundle` wire format is versioned separately via `QUERY_BUNDLE_VERSION`. When the bundle format changes, bump this constant and update any server-side bundle-loading code that validates the version.

---

## HTTP client

`src/lib.rs` contains the `Client` (alias `SparrowDBClient`) that sends queries to a running SparrowDB instance.

- Default URL: `http://localhost:6969`
- All queries go to `POST /v1/query` (dynamic) or `POST /v1/query/<name>` (stored)
- Optional API key is sent as `Bearer` token
- Response bodies are deserialized with `sonic_rs`

Builder pattern:
```rust
let client = Client::new(None)?;  // uses localhost:6969
let result: MyType = client
    .query()
    .dynamic_query(my_query_fn("arg"))
    .send()
    .await?;
```

---

## Updating the SDK after API changes

If the server's query protocol changes (new step type, new wire field, changed serialization):

1. Update the relevant AST types and builder methods in `src/dsl.rs`.
2. If the `QueryBundle` format changes, bump `QUERY_BUNDLE_VERSION` in `src/query_generator.rs`.
3. Run the SDK tests to verify round-trip serialization:
   ```bash
   cargo test --package sparrow-sdk
   ```
4. Update the client in `src/lib.rs` if new HTTP headers or endpoint paths are needed.
5. If `helix-dsl-macros` needs changes (new parameter types, new macro syntax), update that crate and bump its version in `Cargo.toml`.
