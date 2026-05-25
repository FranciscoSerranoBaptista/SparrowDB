---
skill: migration
type: workflow
trigger: >
  Use when changing the schema of a running SparrowDB instance
  (new field, type change, rename, new node/edge/vector type) or
  when ingesting data from an external source (CSV, JSON, Parquet).
entry_point: "Step 1 — Classify the change"
exits:
  - querying.md    # verify data after migration
  - debugging.md   # if migration fails
related:
  - docs/HQL.md
  - docs/import.md
  - docs/HTTP_API.md
---

# SparrowDB — Migration Workflow

---

## Step 1 — Classify the change

```
┌─ Schema change ─────────────────────────────────────────────────┐
│  New field, type change, rename, new node/edge/vector type       │
│  → Follow the Schema Migration path (Steps 2–6)                  │
└─────────────────────────────────────────────────────────────────┘

┌─ Data ingest ────────────────────────────────────────────────────┐
│  Loading CSV / JSON / Parquet from an external source            │
│  → Follow the Bulk Import path (Steps 7–9)                       │
└─────────────────────────────────────────────────────────────────┘
```

---

## Schema Migration Path

### Step 2 — Snapshot first (always)

```bash
sparrow data snapshot
```

Hot-copies the live database to a directory. Safe to run against a running instance.
Do this before every schema change. If migration fails you can restore with:

```bash
sparrow data restore          # restore most recent snapshot
sparrow data restore --force  # overwrite without confirmation
```

Clone a snapshot to a new location:
```bash
sparrow data clone
```

---

### Step 3 — Write the migration block

Schema versions are declared with `schema::N { }` blocks. Migrations map one
version to the next using `MIGRATION schema::N => schema::M { }`:

```hql
schema::1 {
    N::User {
        name:  String,
        email: String,
    }
}

MIGRATION schema::1 => schema::2 {
    N::User => N::User {
        full_name: name,
        email: email,
        status: "active",
        created_at: NOW,
    }
}

schema::2 {
    N::User {
        full_name:  String,
        email:      String,
        status:     String DEFAULT "active",
        created_at: Date   DEFAULT NOW,
    }
}
```

**Supported field transforms inside a migration block:**

| Transform | Syntax | Notes |
|-----------|--------|-------|
| Identity (no change) | `fieldName: fieldName` | Copies value unchanged |
| Rename | `newName: oldName` | Copies old value to new name |
| Type cast | `newName: oldName AS TargetType` | Must be a safe widening cast |
| Literal default | `fieldName: "value"` | Applied to all existing nodes |
| Timestamp default | `fieldName: NOW` | Sets current UTC time on all existing nodes |
| Drop type | `N::OldType => _::` | Maps old type to anonymous (effectively drops it) |

**Changing type names** (e.g. User → Person):
```hql
MIGRATION schema::1 => schema::2 {
    N::User => N::Person {
        name: name,
        birthYear: age AS I32,
    }
    E::Follows => E::Follows { Properties: { weight: 1 } }
}
```

> **Schema version numbers must be contiguous.** A gap (e.g. schema::1 → schema::3)
> causes a compile error.

---

### Step 4 — Validate without deploying

```bash
sparrow check
```

The HQL compiler reports errors with ariadne formatting (`file:line:col` with
underlines). Fix all errors before proceeding.

When running from source:
```bash
cargo run -p sparrow-cli -- check
```

---

### Step 5 — Deploy

Always test in a non-production instance first:

```bash
sparrow push dev    # compile + deploy to dev instance
```

Verify (see Step 6), then promote:

```bash
sparrow push prod   # deploy to production instance
```

---

### Step 6 — Verify the migration

```bash
# Schema reflects new types and fields
curl -H "x-api-key: $TOKEN" http://localhost:6969/introspect

# Spot-check a migrated node (dev-instance feature required)
curl -X POST -H "x-api-key: $TOKEN" \
     -d '{"id": "<known-node-id>"}' \
     http://localhost:6969/node_details
```

Check `/diagnostics` to confirm node/edge counts look correct:
```bash
curl -H "x-api-key: $TOKEN" http://localhost:6969/diagnostics
```

✅ `/introspect` shows new schema, node data looks correct → done
❌ Error or unexpected data → `debugging.md`

---

## Bulk Import Path

### Step 7 — Prepare a query that accepts import rows

Each row in your data file maps to the parameters of an HQL query.
Example for a CSV with columns `name,email`:

```hql
QUERY CreateUser(name: String, email: String) =>
    user <- AddN<User>({ name: name, email: email })
    RETURN user
```

Use `UpsertN` instead of `AddN` if the source data may contain duplicate IDs:
```hql
QUERY UpsertUser(name: String, email: String) =>
    user <- UpsertN<User>({ name: name, email: email })
    RETURN user
```

---

### Step 8 — Run the import

```bash
sparrow import users.csv      --query CreateUser
sparrow import products.json  --query CreateProduct
sparrow import events.parquet --query ImportEvent
```

Key flags (verify against `sparrow import --help`):
- `--workers N` / `-w N` — parallel worker count (default 8)
- `--dry-run` — parse the file and print a preview without writing data
- `--token <api-key>` — auth token (or set `SPARROW_TOKEN` env var)
- `--format <json|csv|parquet>` / `-f` — override format detection
- `--on-error <continue|abort>` — what to do when a record fails (default: `continue`)
- `--query-column <col>` / `-c` — column whose value is the query name for that row (for mixed-type files)
- `--target <url>` / `-t` — SparrowDB base URL (default: `http://localhost:6969`)

Supported formats: **CSV** (with header row), **JSON** (array of objects), **Parquet**

---

### Step 9 — Verify the import

```bash
curl -H "x-api-key: $TOKEN" http://localhost:6969/diagnostics
```

`nodes`, `edges`, and `vectors.total` counts should reflect the imported data.

For more import options and format details see **`docs/import.md`**.

---

## Single-Writer Invariant

LMDB enforces one write transaction at the OS level. The SparrowDB gateway mirrors
this in Rust: **all mutations route through a single writer thread in `WorkerPool`**.

This means:
- Schema migrations are applied sequentially — never run two `sparrow push` operations against the same instance concurrently
- Do not open a `write_txn()` outside of the writer thread path in any custom code
- Snapshot (`sparrow data snapshot`) is a read-only hot copy — safe to run while the instance is live
- Import workers (`--workers N`) increase HTTP concurrency, but writes inside the server still serialize through the single writer

---

## Common Failure Modes

| Symptom | Cause | Fix |
|---------|-------|-----|
| Compile error on migration block | Version gap or type mismatch | Read ariadne output; ensure contiguous version numbers |
| `VECTOR_ERROR` after schema change | Vector dimension changed between schema versions | Drop the old vector type, re-embed into a fresh `vector(N)` type with the correct dimension |
| `GRAPH_ERROR` on import | `AddN` duplicate ID | Switch to `UpsertN` in the import query |
| Import rows silently skipped | Type mismatch in column | Run with `--dry-run` first; check column names match query params exactly |
| Restore fails | Snapshot taken during heavy write load | Take snapshots between bulk operations, not during them |

---

*Full HQL migration syntax → `docs/HQL.md`*
*Import format details → `docs/import.md`*
*Something broke → `docs/skills/debugging.md`*
