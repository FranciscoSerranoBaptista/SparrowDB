<div align="center">

<picture>
  <img src="/assets/full_logo.png" alt="SparrowDB Logo" width="300">
</picture>

**SparrowDB** — an open-source graph-vector database built from scratch in Rust.

<h3>
  <a href="https://discord.gg/2stgMPr5BD">Discord</a>
</h3>

[![License](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)

</div>

---

SparrowDB is a database that makes it easy to build all the components needed for an AI application in a single platform.

You no longer need a separate application DB, vector DB, graph DB, or application layers to manage multiple storage locations. SparrowDB combines graph traversal, vector similarity search, BM25 keyword search, and an MCP server into a single embeddable database written in Rust.

SparrowDB primarily operates with a **graph + vector** data model, but it also supports KV, document, and relational-style data.

---

## Key Features

| Feature | Description |
|---|---|
| **Graph + Vector in one** | Define nodes and edges with HQL, then attach embeddings to any node — no separate vector store needed. |
| **Built-in MCP tools** | Native Model Context Protocol support so AI agents can discover data and walk the graph without writing raw queries. |
| **Built-in embeddings** | Use `Embed()` in HQL to vectorize text at write time — no pre-processing pipeline required. |
| **Dual storage backends** | Choose LMDB (default, zero-copy reads) or RocksDB (high-throughput writes) at startup via a feature flag. |
| **Runtime HQL interpreter** | POST raw HQL to `/__hql_runtime_eval` for dynamic query execution without a recompile. |
| **BM25 + vector hybrid search** | Combine keyword and semantic search in a single traversal step. |
| **Diagnostics endpoint** | `GET /diagnostics` returns node/edge/vector counts and entry point health at a glance. |
| **Data management CLI** | `snapshot`, `clone`, and `restore` commands for live database backups without downtime. |

---

## Getting Started

### Install

```bash
cargo install sparrow-cli
```

### Initialize a project

```bash
sparrow init my-project
cd my-project
```

This creates:
```
my-project/
  sparrow.toml        # instance configuration
  queries/
    schema.hx         # node and edge type definitions
    queries.hx        # query definitions
  .sparrow/           # local instance state (git-ignored)
```

### Define a schema (`queries/schema.hx`)

```
N::User {
    name: String,
    bio:  String,
}

E::Follows {
    From: User,
    To:   User,
}
```

### Write a query (`queries/queries.hx`)

```
QUERY GetUser(id: ID) =>
    user <- N<User>(id)
    RETURN user

QUERY SimilarUsers(id: ID, limit: Int) =>
    user     <- N<User>(id)
    similar  <- SearchVector<User>(user.bio, limit)
    RETURN similar
```

### Start a local instance

```bash
sparrow push dev     # compile and launch the dev instance
```

### Check your schema compiles

```bash
sparrow check
```

---

## CLI Reference

| Command | Description |
|---|---|
| `sparrow init [path]` | Scaffold a new SparrowDB project |
| `sparrow push [instance]` | Compile and deploy to a local instance |
| `sparrow check` | Validate schema and queries without deploying |
| `sparrow run` | Start the database server directly (no container) |
| `sparrow data snapshot` | Hot-copy the live database to a directory |
| `sparrow data clone` | Copy an existing snapshot |
| `sparrow data restore [--force]` | Restore from a snapshot |
| `sparrow metrics [basic\|full\|off\|status]` | Configure anonymous telemetry |

---

## Configuration (`sparrow.toml`)

```toml
[project]
name    = "my-project"
queries = "queries"

[local.dev]
port             = 6969
build_mode       = "dev"
storage_backend  = "lmdb"   # or "rocks"
```

---

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `SPARROW_DATA_DIR` | `~/.sparrow/` | Override the data directory |
| `SPARROW_HOME` | `~/.sparrow/` | Override the config/cache home |
| `SPARROW_CACHE_DIR` | `~/.sparrow/repo` | Override the build cache |
| `SPARROW_RUNTIME_EVAL` | unset | Enable `/__hql_runtime_eval` endpoint when set |

---

## Architecture

```
sparrow-cli         → CLI (init, push, check, run, data, metrics …)
sparrow-core        → Core engine (lib name: sparrow_db)
  sparrow_engine    → Graph/vector traversal ops (lmdb + rocks backends)
  sparrow_gateway   → HTTP API, MCP server, runtime HQL eval
  sparrowc          → HQL compiler (parser → IR → codegen)
sparrow-container   → Docker container runtime entry point
sparrow-chef        → One-shot bootstrap tool for coding agents
sparrow-memory      → Episodic memory layer for AI agents (embedded)
sparrow-macros      → Proc-derive macros (handler registration)
sparrow-metrics     → Optional anonymous telemetry
sdks/rust           → Rust client SDK (package: sparrow-sdk)
sdks/ts             → TypeScript query-builder DSL (package: sparrow-sdk)
```

---

## Storage Backends

SparrowDB ships with two storage backends selectable at compile time:

| Backend | Feature flag | Strengths |
|---|---|---|
| **LMDB** (default) | `lmdb` | Zero-copy reads, crash-safe, low latency |
| **RocksDB** | `rocks` | High write throughput, compaction, column families |

Switch backends by setting `storage_backend` in `sparrow.toml`.

---

## Development

```bash
git clone https://github.com/YOUR_ORG/SparrowDB
cd SparrowDB
cargo build
cargo test
```

Run only the fast unit tests:

```bash
cargo test -p sparrow-cli
cargo test -p sparrow-core
```

---

## License

SparrowDB is released under the [AGPL-3.0 License](LICENSE).
