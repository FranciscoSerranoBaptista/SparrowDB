# Schema Migrations Design

**Date:** 2026-05-22
**Status:** Approved
**Scope:** Issues #1–#6 — migration state tracking, schema version metadata, crash-safe runner, write-path persistence, generated code integration, CLI surface, and explicit deferral of down migrations.

---

## Problem statement

SparrowDB has three layers of migration infrastructure today:

1. **Storage-level** (`storage_migration.rs`) — auto-runs on startup; handles vector endianness and BM25 index rebuilds. Solid, no changes needed.
2. **CLI v1→v2** (`commands/migrate.rs`) — one-time project-format wizard. Done.
3. **HQL schema versioning** — the compiler validates and generates transition functions from `migration v1 -> v2 { ... }` blocks; `VersionInfo` lazily upgrades nodes/edges on read.

Six gaps make the current HQL migration system unsuitable for production:

| # | Gap |
|---|-----|
| 1 | No migration state tracking — no record of what has been applied |
| 2 | No schema version in DB metadata — can't detect version mismatch on startup |
| 3 | Lazy-only upgrades never persist to disk; no crash recovery for bulk runs |
| 4 | No CLI surface (`migrate status`, `apply`) |
| 5 | No integration path from compiler-generated functions into the binary |
| 6 | No down migrations (silent gap, not documented) |

---

## Design

### Architecture overview

| Issue | Component | Location |
|---|---|---|
| #1 | `_migrations_log` LMDB sub-database | `SparrowGraphStorage` |
| #2 | `StorageMetadata::WithSchemaVersion` variant | `storage_core/metadata.rs` |
| #3 | `MigrationRunner` — batched scan, idempotent, crash-safe | `storage_core/storage_migration.rs` |
| #4 | `sparrow migrate status/apply/list` | `sparrow-cli/commands/migrate.rs` |
| #5 | `sparrow build` emits `migrations.rs`; `sparrow-container` includes it | `sparrow-cli/commands/build.rs` + `sparrow-container` |
| #6 | `reversible: bool = false`, `down_fn: Option<…> = None`; documented | `version_info.rs` + CLI help text |

**Core invariant:** `node.version` / `edge.version` stored on disk is the ground truth for what data is at. `_migrations_log` is the ground truth for what the runner has done. They can be reconciled independently — if they disagree, the node version wins.

**Version numbering:** HQL schema version names (e.g., `"v1"`, `"v2"`) are mapped to `u8` ordinals by the compiler in declaration order starting at 1. `"v1"` → `1`, `"v2"` → `2`, etc. `StorageMetadata::WithSchemaVersion` stores the HQL string name for human readability; nodes/edges store the `u8` ordinal for compact on-disk representation.

**Startup sequence (after existing storage migrations):**

Schema migrations run after `migrate()` returns and before `WorkerPool` / `SparrowGateway` are initialised. At this point there are no concurrent readers or writers, so `run_schema_migrations` opens `write_txn()` directly — it does not go through the `WorkerPool` channel.

1. Read `_migrations_log` and collect `inventory::collect!(TransitionSubmission)` registrations
2. Sort transitions into a chain by `from_version → to_version`; validate chain is contiguous
3. Detect any `InProgress` or absent entries → run them directly via `write_txn()`
4. Update `StorageMetadata` to `WithSchemaVersion { schema_version: latest }`

Normal restarts with no pending migrations: one read-txn to check the log; zero overhead.

---

### Section 1 — Migration state table (issue #1)

New LMDB named database `_migrations_log` opened in `SparrowGraphStorage` alongside `nodes_db`, `edges_db`, etc.

**Key:** migration name as UTF-8 bytes — e.g., `b"User_v1_v2"`.

**Value:** bincode-serialized `MigrationRecord`:

```rust
pub struct MigrationRecord {
    pub applied_at: u64,        // unix timestamp; 0 when InProgress
    pub checksum: u64,          // hash of transition fn body, embedded at build time
    pub status: MigrationStatus,
    pub reversible: bool,       // always false in v1; field reserved for down migrations
}

pub enum MigrationStatus {
    InProgress,
    Complete,
}
```

**Checksum behaviour (v1):** if a `Complete` record's checksum does not match the binary's compiled checksum, log a warning and skip. Re-running on checksum mismatch is a v2 concern.

---

### Section 2 — Schema version in metadata (issue #2)

`StorageMetadata` gains a new final variant that extends the existing chain:

```rust
pub enum StorageMetadata {
    PreMetadata,
    VectorNativeEndianness { vector_endianness: VectorEndianness },
    WithSchemaVersion {
        vector_endianness: VectorEndianness,
        schema_version: String,   // name of latest fully-applied HQL schema version
    },
}
```

A database that has only `VectorNativeEndianness` on disk (pre-migration-system) is treated as `schema_version = "v1"` — the implicit starting version. The existing chain-migration loop in `migrate()` is extended to handle the new variant.

---

### Section 3 — Migration runner (issue #3)

The runner is a new function `run_schema_migrations(storage, version_info)` called in the startup sequence immediately after `migrate()` returns, before `WorkerPool` is constructed.

**Discovery:**

1. Collect all `TransitionSubmission`s from `inventory::collect!`
2. Sort into chain order (`from_version → to_version`)
3. Validate the chain is contiguous — if there is a gap, return an error and halt startup rather than apply partial migrations
4. Cross-reference against `_migrations_log`:
   - `Complete` + matching checksum → skip
   - `Complete` + mismatched checksum → warn, skip
   - `InProgress` → resume
   - Absent → insert `InProgress`, then run

**Execution (per transition):**

1. Open a read txn; collect all node IDs where `node.version == from_version` in batches of 1024
2. Drop the read txn
3. For each batch: open a write txn → read each node → apply `transition.func(props)` → write back at `version = to_version` → commit
4. Write an `InProgress` record to `_migrations_log` after each committed batch (best-effort checkpoint)
5. Repeat for edges
6. Mark `Complete` in `_migrations_log`; update `StorageMetadata::WithSchemaVersion`

**Crash recovery:** `node.version` is the idempotency guard. On restart after a crash, the runner sees `InProgress` and re-scans from the beginning. Nodes already at `to_version` are skipped by the `version == from_version` filter. No double-migration risk. A stored checkpoint ID (last processed node ID) can be added later as a pure performance optimisation without changing the correctness model.

---

### Section 4 — Write-path persistence & drain (issue #3, hybrid)

Three-layer drain strategy:

| Layer | Mechanism | When it fires |
|---|---|---|
| Bulk runner | Full scan at startup | On any pending migration |
| Write-path upgrade | Mutation read-modify-write | Any time an existing node is written |
| Read-path upgrade | `upgrade_to_node_latest` in-memory | Every read — correctness fallback only |

**Write-path upgrade:** the writer thread already performs a read-modify-write cycle for mutations (`SetProperty`, edge additions, etc.). The node is passed through `VersionInfo::upgrade_to_node_latest` before the write, so the persisted result is always at `version = latest`. This is a natural consequence of the existing write cycle — no new coordination required.

**Read-path upgrade stays in-memory only.** Read workers must never open write transactions (LMDB single-writer invariant). The in-memory upgrade ensures callers always see the correct version; persistence of cold nodes is delegated to the bulk runner.

**Interaction between layers:** if the runner is mid-batch and a concurrent write upgrades a node in the same batch, the runner's subsequent write of that node is a no-op in effect (writes `to_version` over `to_version`). Idempotent.

---

### Section 5 — Generated code integration (issue #5)

**Current gap:** the HQL compiler generates `GeneratedMigration` structs and formats them as Rust functions with `#[migration(...)]` attributes, but there is no path from that output into the compiled binary.

**Fix:** `sparrow build` gains a migration-emission phase before the Docker build:

1. Compile all HQL files → collect all `GeneratedMigration` outputs
2. Emit `.sparrow/<instance>/generated/migrations.rs`
3. `sparrow-container/build.rs` copies this file into `OUT_DIR`
4. `sparrow-container` includes it: `include!(concat!(env!("OUT_DIR"), "/migrations.rs"))`

**Emitted file format:**

```rust
// generated — do not edit
use sparrow_db::sparrow_engine::storage_core::version_info::{Transition, TransitionSubmission};
use sparrow_db::utils::properties::ImmutablePropertiesMap;

pub fn migration_user_v1_v2(props: ImmutablePropertiesMap) -> ImmutablePropertiesMap {
    // ... generated field remappings via field_addition_from_old_field! etc.
}

inventory::submit! {
    TransitionSubmission(Transition::new("User", 1, 2, migration_user_v1_v2))
}

pub const MIGRATION_USER_V1_V2_CHECKSUM: u64 = 0xdeadbeef_cafebabe; // computed at build time
```

If no migrations are declared, `migrations.rs` is emitted as an empty file. The build always succeeds.

**Checksum computation:** a `build.rs` step hashes the generated function body (not the full file — strip the checksum constant itself) using a stable hash (e.g., `FxHasher` or `xxhash`). The constant is embedded so the runner can read it without re-hashing.

---

### Section 6 — CLI surface & down migrations (issues #4 and #6)

**Rename:** `sparrow migrate` (currently the v1→v2 project wizard) is renamed to `sparrow upgrade`. It is a one-time operation; the name `migrate` is reserved for the ongoing schema migration workflow.

**New `sparrow migrate` subcommands:**

```
sparrow migrate status [instance]
```
Reads `_migrations_log` and prints each registered transition with status, `applied_at`, and checksum match. Example:
```
User      v1 → v2   complete   2026-05-22 09:14  ✓
Post      v1 → v2   complete   2026-05-22 09:14  ✓
Comment   v1 → v2   pending    —
```

```
sparrow migrate apply [instance] [--dry-run]
```
Explicitly runs pending migrations outside of startup. `--dry-run` prints what would run and estimated node counts without touching data. Internally calls the same `run_schema_migrations` path used at startup.

```
sparrow migrate list [instance]
```
Lists all transitions compiled into the running binary (from `inventory::collect!`), whether or not applied. Confirms a build picked up the correct `migrations.rs`.

**Down migrations (deliberately deferred):**

`Transition` gains `down_fn: Option<fn(ImmutablePropertiesMap) -> ImmutablePropertiesMap> = None`. `MigrationRecord` carries `reversible: bool = false`. Both are wired but never populated in v1. The CLI help text documents the gap explicitly:

```
Down migrations are not supported in this version.
To roll back a schema change, restore from backup before applying the migration.
```

---

## What is not in scope

- Re-running migrations on checksum mismatch (v2)
- Stored checkpoint IDs for faster crash-resume (v2 performance optimisation)
- Down migrations / rollback (v2)
- Migration between different item types in HQL (e.g., Node → Edge) — already flagged with TODO in `migration_validation.rs`
- Cloud / distributed migration coordination

---

## Implementation sequence

Follow the issue ordering — each step is a prerequisite for the next:

1. `MigrationRecord` + `_migrations_log` sub-database in `SparrowGraphStorage`
2. `StorageMetadata::WithSchemaVersion` variant + chain extension
3. `run_schema_migrations` runner + crash-safe batched scan
4. Write-path upgrade in the writer thread mutation cycle
5. `sparrow build` migration-emission phase + `sparrow-container` `include!` wiring
6. `sparrow migrate status/apply/list` CLI + rename `migrate` → `upgrade` + down-migration stubs
