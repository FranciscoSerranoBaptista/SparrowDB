# sparrow-container

Docker container runtime wrapper for SparrowDB. This crate is the compiled binary that runs inside the Docker image: it initialises the graph engine, registers handlers and MCP tools, and starts the Axum HTTP gateway.

## Build

```bash
# default (LMDB backend)
cargo build -p sparrow-container --release

# with dev-instance flag (disables API key checks)
cargo build -p sparrow-container --features dev
```

## Run

Normally started by the Docker image entry point. To run locally:

```bash
SPARROW_DATA_DIR=/tmp/sparrow-data cargo run -p sparrow-container
```

The process reads `SPARROW_DATA_DIR` and appends `/user` for the actual database path. If `SPARROW_DATA_DIR` is unset it defaults to `~/.sparrow/user`.

## Feature flags

| Feature | Description |
|---|---|
| `lmdb` (default) | Use the LMDB storage backend (`sparrow-core/lmdb`) |
| `dev` | Development instance — enables `sparrow-core/dev-instance` (relaxes API key enforcement) |
| `production` | Production mode — enables `sparrow-core/production` (strict API key verification) |

## Key files

| File | Description |
|---|---|
| `src/main.rs` | Entry point: tracing setup, engine init, gateway start |
| `src/queries.rs` | Generated query registrations for this container build |
