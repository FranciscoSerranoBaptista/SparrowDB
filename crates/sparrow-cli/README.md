# sparrow-cli

The `sparrow` command-line tool. Manages the full project lifecycle: scaffolding, compiling and deploying HQL schemas, running the database, managing data snapshots, and configuring telemetry.

> `sparrow run` starts the database server directly and does not require Docker. `sparrow push` compiles and deploys to a containerised dev instance and does require Docker.

## Install

```bash
cargo install sparrow-cli
```

## Build from source

```bash
cargo build -p sparrow-cli --release
# binary at: target/release/sparrow
```

## Test

```bash
cargo test -p sparrow-cli
```

## Commands

| Command | Description |
|---|---|
| `sparrow init [path]` | Scaffold a new project (creates `sparrow.toml`, `db/schema.hx`, `db/queries.hx`) |
| `sparrow push [instance]` | Compile schema and deploy to a local Docker dev instance |
| `sparrow check` | Validate schema and queries without deploying |
| `sparrow run` | Start the database server directly (no container) |
| `sparrow data snapshot` | Hot-copy the live database to a directory |
| `sparrow data clone` | Copy an existing snapshot |
| `sparrow data restore [--force]` | Restore from a snapshot |
| `sparrow metrics [basic\|full\|off\|status]` | Configure anonymous telemetry |

## Error handling

Recoverable/library errors use `thiserror::Error` (config, project, port). CLI commands return `eyre::Result` and render errors for consistent output.

## Feature flags

| Feature | Description |
|---|---|
| `normal` (default) | Includes `sparrow-core/server` |
| `ingestion` | Includes `sparrow-core/full` for bulk data ingestion paths |
