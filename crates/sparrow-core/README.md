# sparrow-core

The core SparrowDB database engine. Contains the graph/vector traversal layer (`sparrow_engine`), the HTTP/MCP gateway (`sparrow_gateway`), the HQL compiler (`sparrowc`), and shared protocol types.

> **Import note:** the library name is `sparrow_db` (not `sparrow_core`). Add it to your `Cargo.toml` as `sparrow-core` and import it as `use sparrow_db::...`.

## Build

```bash
# default (LMDB backend, server features)
cargo build -p sparrow-core

# with RocksDB backend
cargo build -p sparrow-core --no-default-features --features rocks

# all features
cargo build -p sparrow-core --features full
```

## Test

```bash
cargo test -p sparrow-core
```

## Key directories

| Path | Contents |
|---|---|
| `src/sparrow_engine/` | Storage backends (LMDB/RocksDB), graph traversal, vector and BM25 search |
| `src/sparrow_gateway/` | Axum HTTP server, MCP handler, `/__hql_runtime_eval`, `/diagnostics` |
| `src/sparrowc/` | HQL compiler: parser (pest grammar), IR, codegen (`compiler` feature) |
| `src/protocol/` | Shared wire types for queries and responses |
| `src/grammar.pest` | PEG grammar for HQL |

## Feature flags

| Feature | Description |
|---|---|
| `lmdb` (default) | Enable LMDB storage backend via heed3 |
| `compiler` | Include the HQL parser and compiler |
| `vectors` | Enable vector/embedding support (`cosine` + `url`) |
| `server` | Full server build: `build` + `compiler` + `vectors` + `reqwest` |
| `dev-instance` | Marker flag for development instances |
| `production` | Enable API key verification (`api-key`) |
| `bench` | Include Polars for benchmark analytics |
| `debug-output` | Enable verbose macro debug output |
| `full` | `build` + `compiler` + `vectors` (no `reqwest`) |
