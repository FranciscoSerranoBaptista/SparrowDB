---
skill: setup
type: workflow
trigger: >
  Use when initialising a new SparrowDB project, configuring an
  instance, or onboarding into an existing project for the first time.
entry_point: "Step 1 — Choose your setup path"
exits:
  - querying.md   # once the instance is live and verified
  - debugging.md  # if any step fails
related:
  - docs/auth.md
  - docs/HTTP_API.md
---

# SparrowDB — Setup Workflow

---

## Step 1 — Choose your setup path

```
┌─ AI agent / fastest start ──────────────────────────────────────┐
│  Use sparrow-chef (Step 1a)                                      │
│  → scaffolds project, starts Docker, seeds example data          │
│  → skip to Step 5 when done                                      │
└─────────────────────────────────────────────────────────────────┘

┌─ Manual / full control ──────────────────────────────────────────┐
│  Use the sparrow CLI (Step 1b → 2 → 3 → 4)                      │
└─────────────────────────────────────────────────────────────────┘
```

---

## Step 1a — Chef path (zero-friction)

```bash
cargo install sparrow-chef   # first time only
sparrow-chef cook --auto     # interactive: sparrow-chef chef
```

Chef will:
1. Scaffold a new project directory
2. Pull the SparrowDB Docker image
3. Start the database container
4. Seed it with example schema and data
5. Write `SPARROWDB_CHEF_PROMPT.md` — load this into your agent's context

→ Jump to **Step 5 — Verify**.

---

## Step 1b — CLI path

```bash
cargo install sparrow-cli    # first time only
sparrow init <project-name>
cd <project-name>
```

Creates:
```
<project-name>/
  sparrow.toml       ← project configuration
  queries/           ← .hx schema and query files go here
  .sparrow/          ← build cache, git-ignored
```

---

## Step 2 — Configure `sparrow.toml`

Minimal configuration:

```toml
[project]
name = "my-project"
queries = "queries"

[local.dev]
port = 6969
build_mode = "dev"
storage_backend = "lmdb"
```

`storage_backend` options:
- `lmdb` — zero-copy reads, crash-safe, single OS-level writer. **Default. Use this unless write throughput is the bottleneck.**
- Check `docs/HTTP_API.md` for additional backend options and when to use them.

---

## Step 3 — Write your schema

Create `queries/schema.hx`:

```hql
N::User {
    name:  String,
    email: String UNIQUE INDEX,
    age:   U32    DEFAULT 0,
}

E::Follows {
    From: User,
    To:   User,
}

V::Document {
    content:   String,
    embedding: vector(1536),   ← dimension must match your embedding model
}

QUERY GetUser(user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
```

Validate without deploying:

```bash
sparrow check
```

`sparrow check` runs the HQL compiler and prints ariadne-formatted errors
(`file:line:col` with underlines). Fix all errors before proceeding.

---

## Step 4 — Start the instance

**Without Docker (direct):**
```bash
sparrow run
```

**Via Docker Compose (recommended for dev):**
```bash
sparrow push dev
```

`sparrow push dev` compiles your schema, builds a Docker image, and starts it via
Docker Compose. The Compose project is named `sparrow-{project_name}-{instance_name}`.
Docker or Podman must be running.

---

## Step 5 — Seed an auth token

On a **fresh instance** (no tokens exist), requests succeed without authentication.
Once the **first token is created**, every request requires `x-api-key: <token>`.

**Fast path — auto-seed on startup:**
```bash
export SPARROW_API_KEY=my-secret-token
sparrow run    # or sparrow push dev
```

The instance auto-creates an admin token from `SPARROW_API_KEY` on first boot.

→ For full token lifecycle (create, rotate, revoke, roles) see **`docs/auth.md`**.

---

## Step 6 — Verify

```bash
# Schema loaded correctly
curl http://localhost:6969/introspect

# Instance is healthy, counts are all zero on a fresh DB
curl http://localhost:6969/diagnostics
```

With auth:
```bash
curl -H "x-api-key: my-secret-token" http://localhost:6969/introspect
curl -H "x-api-key: my-secret-token" http://localhost:6969/diagnostics
```

Expected `/diagnostics` response on a fresh DB:
```json
{
  "nodes": 0,
  "edges": 0,
  "vectors": { "total": 0, "active": 0, "soft_deleted": 0 }
}
```

✅ Both return 200 → proceed to `querying.md`
❌ Any error → proceed to `debugging.md`

---

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `SPARROW_PORT` | `6969` | HTTP server port |
| `SPARROW_API_KEY` | unset | Auto-seed admin token on startup |
| `SPARROW_DATA_DIR` | `~/.sparrow/` | Override data/storage directory |
| `SPARROW_DB_MAX_SIZE_GB` | unset | Override LMDB map size in GB |
| `SPARROW_SKIP_BM25_ON_WRITE` | unset | Set to `true` or `1` to disable BM25 index updates during writes |
| `SPARROW_RUNTIME_HQL` | unset | Enable `/__hql_runtime_eval` when set to `true` or `1` |

> **Verify additional env vars** against `crates/sparrow-container/src/main.rs`
> before adding them here — the list above is confirmed from source.

---

## Feature Flags (building from source)

| Flag | Purpose |
|------|---------|
| `lmdb` | LMDB storage backend (default) |
| `compiler` | HQL parser and compiler |
| `vectors` | HNSW index and embedding support |
| `server` | Full HTTP gateway (implies compiler + vectors) |
| `dev-instance` | Debug endpoints (`/node_details`, etc.) |
| `production` | Enables auth enforcement marker |
| `debug-output` | Verbose macro expansion diagnostics |
| `cosine` | Cosine similarity metric for HNSW |

Tests that touch the graph or gateway need both storage and server:
```bash
cargo test --features lmdb,server
```

LMDB stress tests must be serialised:
```bash
cargo test --package sparrow-core --features lmdb -- --test-threads=1
```

---

## Common Failure Modes

| Symptom | Cause | Fix |
|---------|-------|-----|
| `connection refused :6969` | Instance not started | Run `sparrow run` or `sparrow push dev` |
| Port conflict | Something already on 6969 | Set `SPARROW_PORT=6970` or kill the process |
| Docker error on `sparrow push` | Docker daemon not running | Start Docker Desktop / Podman |
| `sparrow check` fails silently | Missing feature flags | Run with `--features lmdb,server` when checking from source |
| Schema compile error | HQL syntax error | Read ariadne output; it points to `file:line:col` |
| `/introspect` returns empty schema | `.hx` file not in `queries/` dir | Check `sparrow.toml` `queries` path |

---

*Token management → `docs/auth.md`*
*Writing queries → `docs/skills/querying.md`*
*Something broke → `docs/skills/debugging.md`*
