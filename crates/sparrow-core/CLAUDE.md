# sparrow-core CLAUDE.md

The database engine crate. Read this before modifying storage, the gateway, or the compiler.

---

## Critical invariant: lib name vs package name

- **`Cargo.toml` `name`**: `sparrow-core`
- **`[lib] name`**: `sparrow_db`

All downstream crates (`sparrow-cli`, `sparrow-container`, `sparrow-memory`) import this crate as `use sparrow_db::...`. If you remove the `[lib]` section the crate name reverts to `sparrow_core` and every import site breaks. Do not remove it.

---

## Directory layout under `src/`

```
src/
  lib.rs                  <- module declarations, global allocator (MiMalloc)
  grammar.pest            <- PEG grammar for the HQL compiler
  protocol/               <- wire types: Request, Response, Format, Value
  sparrow_engine/         <- storage layer (LMDB) and traversal ops
  sparrow_gateway/        <- HTTP server, router, worker pool
    gateway.rs            <- SparrowGateway::run(), axum app construction
    worker_pool/          <- WorkerPool: N read workers + 1 write worker
    router/               <- SparrowRouter: inventory-based handler dispatch
    v1_compat/            <- /v1/query HelixDB compatibility endpoint
    builtin/              <- dev-only built-in query handlers (feature = dev-instance)
    mcp/                  <- MCP (model context protocol) tool support
    embedding_providers/  <- embedding model clients (reqwest-based)
    introspect_schema.rs  <- /introspect GET handler
  sparrowc/               <- HQL compiler (feature = compiler)
  utils/                  <- shared helpers
```

---

## LMDB writer thread

LMDB allows only one write transaction at a time. `WorkerPool` enforces this:

- `N` read workers share a `flume` channel and can open concurrent read transactions.
- **Exactly 1** writer worker receives from a separate `write_tx` channel.

All mutation handlers **must be registered as write routes** so `WorkerPool` dispatches them to the writer. When adding a new endpoint, pass it in `write_routes: Option<HashSet<String>>` when constructing `SparrowGateway`, or mark it with `Handler::new("name", handler_fn, true)` (the `true` flag = is_write).

Never call `storage.graph_env.write_txn()` from a read worker. The LMDB library will deadlock or panic.

---

## v1/query compatibility endpoint

`src/sparrow_gateway/v1_compat/mod.rs` provides `POST /v1/query` as a bridge for callers still using the HelixDB JSON DSL. It translates HelixDB step objects (`NWhere`, `Out`, `In`, `AddN`, `AddE`, `SetProperty`, `Drop`, `VectorSearchNodes`) into SparrowDB traversal calls.

**The `/v1/query` route MUST be registered before `/{*path}` in gateway.rs.** The wildcard handler rejects any path that contains `/`, which `v1/query` does. Registering them in the wrong order causes all v1 traffic to be rejected. This is documented with a comment in `gateway.rs`.

The endpoint peeks at the raw request bytes for the string `"write"` to decide which worker pool channel to use, preserving the single-writer invariant.

---

## sonic_rs 0.5.7 API notes

The codebase uses `sonic-rs = "0.5.7"`. This version has several differences from `serde_json`:

- **No `as_object_mut()`** — mutating a `sonic_rs::Value` in place is not supported. Reconstruct the object instead.
- **Use `sonic_rs::json!{}` macro** — not `object!{}` (that macro does not exist in this version).
- **`sonic_rs::Array` derefs to `[Value]`** — you can iterate over it with `.iter()` or index it, but you must call `.as_array()` on a `Value` to get `Option<&Array>`, then dereference with `&**arr` to get `&[Value]`.
- **`JsonContainerTrait` and `JsonValueTrait`** must be in scope for `.get()`, `.as_str()`, `.as_i64()`, etc., to work on `sonic_rs::Value`.

---

## Vector index (HNSW)

The vector index uses HNSW (Hierarchical Navigable Small World). Key behaviors:

- **Soft delete**: when a node is deleted from the graph, its vector entry is soft-deleted in the HNSW index. The entry is marked invalid but the index structure is not rebuilt. This keeps deletion O(1) but the index can accumulate stale entries over time.
- **Hard delete / compaction**: not yet implemented. A future compaction step will rebuild the index to remove stale entries.
- **Cosine similarity**: enabled by the `cosine` feature (included in the `vectors` feature set).

Do not assume a deleted node's vector has been physically removed from the index. Filter out soft-deleted entries in search results.

---

## Adding a new built-in endpoint

1. Create a new file under `src/sparrow_gateway/builtin/my_endpoint.rs`.
2. Implement the handler function with the signature `fn my_handler(input: HandlerInput) -> Result<Response, GraphError>`.
3. Register it with inventory:
   ```rust
   inventory::submit! {
       HandlerSubmission(Handler::new("my_endpoint_name", my_handler, false /* or true if write */))
   }
   ```
4. If the endpoint should only exist in dev builds, gate the `inventory::submit!` and the import behind `#[cfg(feature = "dev-instance")]`.
5. Add the module to `src/sparrow_gateway/builtin/mod.rs`.
6. For dev-only routes, also add the axum route registration in `gateway.rs` inside the `#[cfg(feature = "dev-instance")]` block.

The `SparrowRouter` picks up `inventory::submit!` registrations at startup automatically — no explicit router.insert() call needed.

---

## Feature flags (sparrow-core)

```
compiler          = pest + pest_derive + ariadne  (HQL parser)
build             = compiler
vectors           = cosine + url  (HNSW + embedding client URLs)
server            = build + compiler + vectors + reqwest  (full gateway)
lmdb              = server + heed3  (storage backend — the default)
dev-instance      = exposes debug query handlers (/nodes-edges, /node-details, etc.)
production        = production build marker (auth enforced via lmdb TokenStore)
bench             = polars  (benchmarking utilities)
debug-output      = verbose macro output
```

The `ariadne` crate must be included when the `compiler` feature is active. Removing it breaks the error-reporting path in the HQL compiler. See the project memory note on this recurring issue.
