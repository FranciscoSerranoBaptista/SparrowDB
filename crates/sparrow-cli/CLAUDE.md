# sparrow-cli CLAUDE.md

The `sparrow` CLI. Read this before adding commands, touching Docker integration, or writing integration tests.

---

## Binary vs library

`Cargo.toml` defines both:
```toml
[lib]
name = "sparrow_cli"
path = "src/lib.rs"

[[bin]]
name = "sparrow"
path = "src/main.rs"
```

The library crate exists so integration tests can import CLI internals directly without going through the binary. All command logic lives in the lib; `main.rs` is a thin entry point.

---

## Commands

Each command is a module under `src/commands/`. The async entry point is conventionally `pub async fn run(...)`.

| Command module | Description |
|---|---|
| `add` | Add a new instance or resource to the project |
| `auth` | Login / logout / API key management |
| `backup` | Backup and restore operations |
| `build` | Compile `.hx` queries and build Docker images |
| `check` | Validate project configuration |
| `cloud_api` | Low-level cloud API calls |
| `compile` | Compile `.hx` query files to Rust code or `queries.json` |
| `config` | Manage workspace / project / cluster config |
| `dashboard` | Start / stop / status of the local dashboard |
| `data` | Snapshot, clone, and restore database directories |
| `delete` | Delete an instance |
| `feedback` | Submit feedback / open a GitHub issue |
| `init` | Initialize a new SparrowDB project |
| `integrations/` | Third-party integrations |
| `logs/` | Log streaming |
| `metrics` | Enable / disable / show metrics |
| `migrate` | Data migration utilities |
| `prune` | Remove stopped containers and dangling volumes |
| `push` | Push a compiled query bundle to an enterprise cluster |
| `restart` | Restart a running instance |
| `run` | Run the server binary directly (no Docker) |
| `start` | Start an instance via Docker |
| `status` | Show instance status |
| `stop` | Stop a running instance |
| `stress` | Run a stress test against an instance |
| `sync` | Sync enterprise cluster state |
| `update` | Self-update the CLI binary |
| `workspace_flow` | Wizard for workspace / project selection |

---

## Project discovery

`src/project.rs` implements `ProjectContext::find_and_load()`. It walks up the directory tree from the current working directory (or an explicit start path) looking for `sparrow.toml`. The first ancestor directory containing `sparrow.toml` becomes the project root.

Instance workspaces live under `<root>/.sparrow/<instance_name>/`. Persistent data volumes live under `<root>/.sparrow/.volumes/<instance_name>/`.

If `sparrow.toml` is not found anywhere in the tree, the command should return a helpful error rather than panicking.

---

## Docker integration

`src/docker.rs` manages Docker and Podman via their CLI (`docker`/`podman` commands). Both runtimes share the same CLI interface so the code works with either.

**CRITICAL: `src/docker.rs` currently uses `std::process::Command`** for synchronous helper functions. If you make any of these functions async, you MUST switch them to `tokio::process::Command`. Using `std::process::Command` inside an async context blocks the Tokio runtime thread and has caused hangs in the past.

`src/commands/run.rs` is an example of doing it correctly — it already imports and uses `tokio::process::Command`.

The compose project naming convention is `sparrow-{project_name}-{instance_name}` (hyphens, not underscores, because Fly.io rejects underscores in instance names).

---

## Feature flags

```
normal    = sparrow-core/server   (default: full gateway + compiler)
ingestion = sparrow-core/full     (adds ingestion-specific features)
```

The default feature is `normal`. Almost all CLI functionality needs `normal`. The `ingestion` feature is for data-loading workflows that need the full sparrow-core feature set.

---

## helix-enterprise-ql dependency

`helix-enterprise-ql = "0.1.1"` is listed in `Cargo.toml` as a legacy external crate. It is part of the enterprise query bundle workflow: when a queries project directory contains a `Cargo.toml`, `sparrow compile` delegates to `cargo run` in that project (calling `run_enterprise_compile()` in `src/commands/compile.rs`). The enterprise project uses `helix-enterprise-ql` to generate `queries.json` from Rust DSL code.

This crate is **planned for replacement** by the native `sparrow-sdk` query generator (`sdks/rust/src/query_generator.rs`). Until that migration is complete, do not remove the dependency.

If `helix-enterprise-ql` causes a build error, check that its version on crates.io is compatible with the current Rust edition and that its transitive dependencies do not conflict.

---

## Integration tests

Integration tests live in `tests/` at the crate root (Cargo convention). They import from `sparrow_cli` (the lib target).

Run CLI tests only:
```bash
cargo test --package sparrow-cli
```

Some integration tests start real Docker containers. They require Docker or Podman to be running and available in `PATH`. These tests are slow and should not be run in CI without a container runtime present.

Tests that access shared state (e.g., a running Docker instance) use `serial_test` to prevent interference.
