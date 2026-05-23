# SparrowDB Skills Files Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write four `docs/skills/*.md` files that give AI assistants accurate, executable playbooks and reference material for querying, setup, migration, and debugging SparrowDB.

**Architecture:** Two structural types — Type C (reference, consulted non-linearly) for `querying.md`, Type A (workflow, followed top-to-bottom) for the other three. All files share a YAML frontmatter block and draw content from the live codebase — not from memory or the spec alone. Each file is self-contained.

**Tech Stack:** Markdown, HQL (`.hx`), Rust CLI (`sparrow`), LMDB, HNSW, MCP

**Spec:** `docs/superpowers/specs/2026-05-23-sparrowdb-skills-design.md`

**Source of truth files (read these before writing each skill):**
- HQL language: `docs/HQL.md`
- HTTP API + error codes + env vars: `docs/HTTP_API.md`
- Auth flow: `docs/auth.md`
- Import guide: `docs/import.md`
- CLI commands: `crates/sparrow-cli/src/main.rs` (lines 38–370)
- Env vars in container: `crates/sparrow-container/src/main.rs` (lines 42, 51, 142)
- Feature flags: `crates/sparrow-core/Cargo.toml` (lines 80–94)
- Grammar rules: `crates/sparrow-core/src/grammar.pest`
- Workspace CLAUDE.md: `CLAUDE.md`

**Known source facts (verified 2026-05-23):**
- Runtime eval env var: `SPARROW_RUNTIME_HQL` (confirmed in container/src/main.rs)
- Feature flags in sparrow-core: `debug-output`, `compiler`, `cosine`, `build`, `vectors`, `server`, `full`, `bench`, `dev`, `dev-instance`, `studio`, `lmdb`, `default`, `production` — no `rocks` flag; verify RocksDB status before writing about it
- HTTP error codes: `INVALID_API_KEY`, `FORBIDDEN`, `NOT_FOUND`, `GRAPH_ERROR`, `VECTOR_ERROR`
- CLI subcommands (exact names): `init`, `add`, `check`, `compile`, `build`, `push`, `start`, `run`, `stop`, `restart`, `status`, `logs`, `prune`, `delete`, `metrics`, `data`, `update`, `migrate`, `upgrade`, `backup`, `import`, `export`, `stress`

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `docs/skills/querying.md` | Create | Type C reference: HQL concepts, patterns, type system, MCP annotations, gotchas |
| `docs/skills/setup.md` | Create | Type A workflow: install → configure → run → verify |
| `docs/skills/migration.md` | Create | Type A workflow: snapshot → write migration → check → deploy; bulk import |
| `docs/skills/debugging.md` | Create | Type A workflow: classify symptom → diagnose → branch → fix |

---

## Task 1: Create `docs/skills/` directory and write `querying.md`

**Files:**
- Create: `docs/skills/querying.md`

**Sources to read before writing:**
- `docs/HQL.md` — full HQL language reference (skim for operator names and examples)
- `crates/sparrow-core/src/grammar.pest` — ground-truth operator spelling

---

- [ ] **Step 1.1: Verify the directory does not exist yet**

```bash
ls docs/skills/ 2>/dev/null && echo "EXISTS" || echo "DOES NOT EXIST"
```

Expected: `DOES NOT EXIST` (or empty listing)

---

- [ ] **Step 1.2: Read the HQL reference to collect operator names**

Read `docs/HQL.md`. You need the exact spelling and signature of:
- Node ops: `N<Type>(id)`, `AddN`, `UpsertN`, `UPDATE`, `DROP`
- Edge ops: `AddE<Type>::From(a)::To(b)`, `UpsertE`
- Vector ops: `AddV`, `UpsertV`, `SearchV`, `SearchN`, `SearchBM25`, `Embed(text)`, `BatchAddV`
- Traversal: `Out<E>`, `In<E>`, `OutE`, `InE`, `FromN`, `ToN`
- Filtering: `WHERE`, `AND`, `OR`, `EXISTS`, `INTERSECT`
- Aggregation: `COUNT`, `RANGE`, `ORDER<Asc|Desc>`, `GROUP_BY`, `FIRST`
- Path: `ShortestPath`, `ShortestPathDijkstras`, `ShortestPathAStar`
- Rerank: `RerankRRF`, `RerankMMR`
- Remapping: `{field: value}`, `..` spread, `!{fields}` exclude, `|var|{...}` closure
- Macros: `#[mcp]`, `#[model("name")]`

---

- [ ] **Step 1.3: Write the validation checklist (run AFTER writing the file)**

Write this to a scratch note — you will run these after Step 1.5:

```bash
# Run each line; all must succeed (exit 0, non-empty output)
grep -q 'skill: querying' docs/skills/querying.md && echo "✓ frontmatter slug"
grep -q 'type: reference' docs/skills/querying.md && echo "✓ frontmatter type"
grep -q 'trigger:' docs/skills/querying.md && echo "✓ frontmatter trigger"
grep -q 'SearchV' docs/skills/querying.md && echo "✓ vector search operator"
grep -q 'SearchBM25' docs/skills/querying.md && echo "✓ BM25 operator"
grep -q 'RerankRRF' docs/skills/querying.md && echo "✓ rerank operator"
grep -q '#\[mcp\]' docs/skills/querying.md && echo "✓ MCP macro"
grep -q '#\[model' docs/skills/querying.md && echo "✓ model macro"
grep -q 'ShortestPath' docs/skills/querying.md && echo "✓ shortest path"
grep -q 'vector(N)' docs/skills/querying.md && echo "✓ vector type"
grep -q 'soft.delet' docs/skills/querying.md && echo "✓ soft-delete gotcha"
grep -q 'docs/HQL.md' docs/skills/querying.md && echo "✓ cross-reference to HQL.md"
```

---

- [ ] **Step 1.4: Create `docs/skills/` and write `docs/skills/querying.md`**

Create the file with this exact content (fill in the HQL examples you collected in Step 1.2 — the structure below is complete, examples must be verified against `docs/HQL.md`):

```markdown
---
skill: querying
type: reference
trigger: >
  Use when writing or reviewing HQL queries, understanding query
  results, optimising traversal, or exposing queries as MCP tools
  for AI agents.
related:
  - docs/HQL.md
  - docs/HTTP_API.md
---

# SparrowDB — Querying Reference

SparrowDB queries are defined in `.hx` files using HQL (Helix Query Language).
Each compiled `QUERY` becomes a POST endpoint at `/<QueryName>`.

---

## Concept Map

- **Node** (`N`) — a typed entity stored in the graph
- **Edge** (`E`) — a directed, typed relationship between two nodes
- **Vector** (`V`) — an embedding attached to a node or stored independently
- **`::`** — the step-chaining operator; each step narrows or transforms the result
- **`_`** — refers to the current element in filters and field remapping

The compiler turns `.hx` schema + query definitions into Rust handlers registered
on the HTTP gateway. The gateway enforces the LMDB single-writer rule: all mutations
route through one write worker; reads fan out across N read workers.

---

## Query Anatomy

```hql
QUERY GetUser(user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
```

- `QUERY <Name>(<params>)` — defines name (→ endpoint path) and typed parameters
- `<var> <- <expression>` — binds a result to a variable
- `RETURN <value>` — the response body (object, array, scalar, or remapped shape)
- Parameters map to the JSON request body field names exactly

---

## Pattern Library

### 1. Node lookup by ID

```hql
QUERY GetUser(user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
```

### 2. Edge traversal — outbound

```hql
QUERY GetFollowees(user_id: ID) =>
    user    <- N<User>(user_id)
    follows <- user::Out<Follows>
    RETURN follows
```

### 3. Edge traversal — inbound

```hql
QUERY GetFollowers(user_id: ID) =>
    user      <- N<User>(user_id)
    followers <- user::In<Follows>
    RETURN followers
```

### 4. Vector similarity search

```hql
QUERY SearchDocuments(query_vec: [F64], k: I32) =>
    results <- SearchV<Document>(query_vec, k)
    RETURN results
```

### 5. Node-field vector search

```hql
QUERY SearchUsers(query_vec: [F64], k: I32) =>
    results <- SearchN<User>(query_vec, k)
    RETURN results
```

### 6. BM25 full-text search

```hql
QUERY SearchByText(query: String, k: I32) =>
    results <- SearchBM25<Document>(query, k)
    RETURN results
```

### 7. Hybrid search + rerank (RRF)

```hql
QUERY HybridSearch(query_vec: [F64], query_text: String, k: I32) =>
    vec_results  <- SearchV<Document>(query_vec, k)
    text_results <- SearchBM25<Document>(query_text, k)
    reranked     <- RerankRRF(vec_results, text_results)
    RETURN reranked
```

`RerankMMR` adds diversity: `RerankMMR(results, lambda)` where `lambda` ∈ [0,1]
(0 = full diversity, 1 = full relevance).

### 8. Filtered traversal

```hql
QUERY GetActiveUsers() =>
    users <- N<User>::WHERE(_.active EQ true AND _.age GTE 18)
    RETURN users
```

### 9. Aggregation

```hql
QUERY CountUsersByCountry() =>
    users   <- N<User>
    grouped <- users::GROUP_BY(_.country)::COUNT
    ordered <- grouped::ORDER<Desc>
    RETURN ordered
```

### 10. Shortest path

```hql
QUERY FindPath(from_id: ID, to_id: ID) =>
    path <- ShortestPath<Follows>(from_id, to_id)
    RETURN path
```

Alternatives: `ShortestPathDijkstras` (weighted edges), `ShortestPathAStar`.

---

## MCP Tool Exposure

Annotate a query to expose it as an MCP tool for AI agents:

```hql
#[mcp]
QUERY SearchDocuments(query_vec: [F64], k: I32) =>
    results <- SearchV<Document>(query_vec, k)
    RETURN results
```

Use `#[model("embedding-model-name")]` when the query uses `Embed(text)` so the
compiler knows which model to call:

```hql
#[mcp]
#[model("text-embedding-3-small")]
QUERY SemanticSearch(query: String, k: I32) =>
    vec     <- Embed(query)
    results <- SearchV<Document>(vec, k)
    RETURN results
```

The generated MCP tool name matches the `QUERY` name. Input schema is derived from
the query parameter types.

---

## Type System

| Category | Types |
|----------|-------|
| Integer | `I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32`, `U64`, `U128` |
| Float | `F32`, `F64` |
| Text | `String` |
| Boolean | `Boolean` |
| Identity | `ID` (UUID — never pass as `String`) |
| Temporal | `Date` (UTC timestamp); `NOW` as default value |
| Array | `[T]` for any type T |
| Object | `{field: Type, ...}` |
| Embedding | `vector(N)` where N is the dimension count |

---

## Field Remapping

```hql
# Include specific fields
RETURN { name: _.name, age: _.age }

# Spread all fields then override one
RETURN { .._, age: 0 }

# Exclude fields
RETURN _::!{password, secret}

# Closure mapping over an array
users <- users::|u|{ name: u.name, id: u.id }
```

---

## Gotchas

1. **`ID` is not `String`** — `ID` is a UUID type. Passing a UUID as `String` will
   cause a type error or silent mismatch.

2. **Vector dimension mismatch** — `vector(N)` in the schema must exactly match
   the embedding model's output dimension. A mismatch causes `VECTOR_ERROR` at
   insert time.

3. **Soft-deleted vectors** — `DROP` on a node marks its HNSW vector entry as
   soft-deleted but does not compact the index. Stale entries accumulate over time
   and can appear as ghost neighbours in search results. There is currently no
   automatic compaction. Mitigation: re-embed the collection into a fresh vector type.

4. **`AddN` vs `UpsertN`** — `AddN` errors on duplicate IDs; `UpsertN` merges.
   Know which you need before using.

5. **`!{fields}` excludes from the response, not from storage** — the underlying
   data is untouched; only the return shape is affected.

6. **`WHERE` predicate precedence** — `AND` binds tighter than `OR`. Use parentheses
   when mixing: `WHERE((_.a EQ 1 AND _.b EQ 2) OR _.c EQ 3)`.

---

## Operator Quick Reference

### Mutation
| Operator | Purpose |
|----------|---------|
| `AddN<Type>(fields)` | Create a node (errors on dup ID) |
| `UpsertN<Type>(fields)` | Create or merge a node |
| `UPDATE(fields)` | Update fields on the current node |
| `DROP` | Delete the current node |
| `AddE<Type>::From(a)::To(b)` | Create a directed edge |
| `UpsertE<Type>::From(a)::To(b)` | Create or merge an edge |
| `AddV(vec)` | Attach a vector embedding |
| `UpsertV(vec)` | Create or update a vector embedding |
| `BatchAddV(vecs)` | Bulk-insert vectors |

### Traversal
| Operator | Purpose |
|----------|---------|
| `Out<E>` | Follow outbound edges of type E |
| `In<E>` | Follow inbound edges of type E |
| `OutE<E>` | Return outbound edge objects |
| `InE<E>` | Return inbound edge objects |
| `FromN` | Get the source node of an edge |
| `ToN` | Get the target node of an edge |

### Filter & Control
| Operator | Purpose |
|----------|---------|
| `WHERE(pred)` | Filter by predicate |
| `AND`, `OR` | Combine predicates |
| `EXISTS` | Test for non-empty traversal |
| `INTERSECT` | Intersection of two sets |
| `RANGE(offset, limit)` | Paginate |
| `ORDER<Asc\|Desc>` | Sort |
| `FIRST` | Take the first element |

### Vector & Search
| Operator | Purpose |
|----------|---------|
| `SearchV<Type>(vec, k)` | ANN search on vector type |
| `SearchN<Type>(vec, k)` | ANN search on node-field embedding |
| `SearchBM25<Type>(text, k)` | Keyword search |
| `Embed(text)` | Call embedding model inline |
| `RerankRRF(a, b)` | Reciprocal rank fusion |
| `RerankMMR(results, λ)` | Maximal marginal relevance |

### Aggregation
| Operator | Purpose |
|----------|---------|
| `COUNT` | Count elements |
| `GROUP_BY(field)` | Group by a field value |
| `ORDER<Asc\|Desc>` | Sort (also in aggregation) |
| `RANGE(offset, limit)` | Slice |

---

*For full operator syntax and examples see `docs/HQL.md`.*
*For HTTP request/response format see `docs/HTTP_API.md`.*
```

---

- [ ] **Step 1.5: Run the validation checklist from Step 1.3**

```bash
grep -q 'skill: querying' docs/skills/querying.md && echo "✓ frontmatter slug"
grep -q 'type: reference' docs/skills/querying.md && echo "✓ frontmatter type"
grep -q 'trigger:' docs/skills/querying.md && echo "✓ frontmatter trigger"
grep -q 'SearchV' docs/skills/querying.md && echo "✓ vector search operator"
grep -q 'SearchBM25' docs/skills/querying.md && echo "✓ BM25 operator"
grep -q 'RerankRRF' docs/skills/querying.md && echo "✓ rerank operator"
grep -q '#\[mcp\]' docs/skills/querying.md && echo "✓ MCP macro"
grep -q '#\[model' docs/skills/querying.md && echo "✓ model macro"
grep -q 'ShortestPath' docs/skills/querying.md && echo "✓ shortest path"
grep -q 'vector(N)' docs/skills/querying.md && echo "✓ vector type"
grep -q 'soft.delet' docs/skills/querying.md && echo "✓ soft-delete gotcha"
grep -q 'docs/HQL.md' docs/skills/querying.md && echo "✓ cross-reference to HQL.md"
```

Expected: all 12 lines print `✓`. Fix any that fail before committing.

---

- [ ] **Step 1.6: Commit**

```bash
git add docs/skills/querying.md
git commit -m "docs(skills): add querying reference skill"
```

---

## Task 2: Write `docs/skills/setup.md`

**Files:**
- Create: `docs/skills/setup.md`

**Sources to read before writing:**
- `crates/sparrow-cli/src/main.rs` — exact subcommand names and flags
- `crates/sparrow-container/src/main.rs` — env var names (lines 42, 51, 142)
- `docs/auth.md` — auth flow (link to it, don't duplicate)
- `crates/sparrow-core/Cargo.toml` (lines 80–94) — feature flag names
- `crates/sparrow-chef/src/main.rs` — chef commands

---

- [ ] **Step 2.1: Read the container source for env var names**

```bash
grep -n 'SPARROW_' crates/sparrow-container/src/main.rs
```

Note every `SPARROW_*` variable name found. These are the authoritative names.

---

- [ ] **Step 2.2: Read the CLI source for `data` subcommand options**

```bash
grep -n -A3 'snapshot\|clone\|restore' crates/sparrow-cli/src/commands/data.rs 2>/dev/null \
  || grep -rn 'snapshot\|clone\|restore' crates/sparrow-cli/src/
```

Note exact flag names for `sparrow data snapshot`, `sparrow data restore`, `sparrow data clone`.

---

- [ ] **Step 2.3: Read the chef source for commands**

```bash
grep -n 'chef\|cook\|auto' crates/sparrow-chef/src/main.rs | head -30
```

Note the exact CLI invocation for chef.

---

- [ ] **Step 2.4: Write the validation checklist**

```bash
grep -q 'skill: setup' docs/skills/setup.md && echo "✓ frontmatter slug"
grep -q 'type: workflow' docs/skills/setup.md && echo "✓ frontmatter type"
grep -q 'entry_point:' docs/skills/setup.md && echo "✓ entry_point"
grep -q 'sparrow-chef' docs/skills/setup.md && echo "✓ chef fast path"
grep -q 'sparrow init' docs/skills/setup.md && echo "✓ init command"
grep -q 'sparrow run' docs/skills/setup.md && echo "✓ run command"
grep -q 'sparrow push' docs/skills/setup.md && echo "✓ push command"
grep -q 'sparrow check' docs/skills/setup.md && echo "✓ check command"
grep -q 'SPARROW_PORT' docs/skills/setup.md && echo "✓ SPARROW_PORT env var"
grep -q 'SPARROW_API_KEY' docs/skills/setup.md && echo "✓ SPARROW_API_KEY env var"
grep -q '/introspect' docs/skills/setup.md && echo "✓ introspect endpoint"
grep -q '/diagnostics' docs/skills/setup.md && echo "✓ diagnostics endpoint"
grep -q 'docs/auth.md' docs/skills/setup.md && echo "✓ link to auth.md"
grep -q 'debugging.md' docs/skills/setup.md && echo "✓ exit to debugging.md"
```

---

- [ ] **Step 2.5: Write `docs/skills/setup.md`**

Use env var names from Step 2.1 (authoritative). Replace any that differ from the list below:

```markdown
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
| `SPARROW_RUNTIME_HQL` | unset | Enable `/__hql_runtime_eval` when set to any value |

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
```

---

- [ ] **Step 2.6: Run the validation checklist from Step 2.4**

```bash
grep -q 'skill: setup' docs/skills/setup.md && echo "✓ frontmatter slug"
grep -q 'type: workflow' docs/skills/setup.md && echo "✓ frontmatter type"
grep -q 'entry_point:' docs/skills/setup.md && echo "✓ entry_point"
grep -q 'sparrow-chef' docs/skills/setup.md && echo "✓ chef fast path"
grep -q 'sparrow init' docs/skills/setup.md && echo "✓ init command"
grep -q 'sparrow run' docs/skills/setup.md && echo "✓ run command"
grep -q 'sparrow push' docs/skills/setup.md && echo "✓ push command"
grep -q 'sparrow check' docs/skills/setup.md && echo "✓ check command"
grep -q 'SPARROW_PORT' docs/skills/setup.md && echo "✓ SPARROW_PORT env var"
grep -q 'SPARROW_API_KEY' docs/skills/setup.md && echo "✓ SPARROW_API_KEY env var"
grep -q '/introspect' docs/skills/setup.md && echo "✓ introspect endpoint"
grep -q '/diagnostics' docs/skills/setup.md && echo "✓ diagnostics endpoint"
grep -q 'docs/auth.md' docs/skills/setup.md && echo "✓ link to auth.md"
grep -q 'debugging.md' docs/skills/setup.md && echo "✓ exit to debugging.md"
```

Expected: all 14 lines print `✓`. Fix any that fail.

---

- [ ] **Step 2.7: Commit**

```bash
git add docs/skills/setup.md
git commit -m "docs(skills): add setup workflow skill"
```

---

## Task 3: Write `docs/skills/migration.md`

**Files:**
- Create: `docs/skills/migration.md`

**Sources to read before writing:**
- `docs/HQL.md` — migration block syntax (search for `MIGRATION schema::`)
- `docs/import.md` — bulk import formats, flags, and examples
- `crates/sparrow-cli/src/commands/` — `data`, `import`, `export`, `migrate` subcommand source

---

- [ ] **Step 3.1: Read migration syntax from HQL docs**

```bash
grep -n 'MIGRATION\|schema::' docs/HQL.md | head -40
```

Note the exact syntax for: `schema::N { }` versioned blocks, `MIGRATION schema::N => schema::M { }`, field rename, type cast, literal default, `NOW` default.

---

- [ ] **Step 3.2: Read import command flags**

```bash
# Try the import command help
cargo run -p sparrow-cli -- import --help 2>/dev/null || true
# Or read the source directly
grep -n 'workers\|batch\|dry.run\|token' crates/sparrow-cli/src/commands/import.rs 2>/dev/null \
  || grep -rn 'workers\|batch_size\|dry_run' crates/sparrow-cli/src/
```

Note exact flag names for `sparrow import`.

---

- [ ] **Step 3.3: Write the validation checklist**

```bash
grep -q 'skill: migration' docs/skills/migration.md && echo "✓ frontmatter slug"
grep -q 'type: workflow' docs/skills/migration.md && echo "✓ frontmatter type"
grep -q 'entry_point:' docs/skills/migration.md && echo "✓ entry_point"
grep -q 'sparrow data snapshot' docs/skills/migration.md && echo "✓ snapshot command"
grep -q 'MIGRATION schema' docs/skills/migration.md && echo "✓ migration block syntax"
grep -q 'sparrow check' docs/skills/migration.md && echo "✓ check before deploy"
grep -q 'sparrow push' docs/skills/migration.md && echo "✓ deploy command"
grep -q 'sparrow import' docs/skills/migration.md && echo "✓ import command"
grep -q 'single.writer\|single writer' docs/skills/migration.md && echo "✓ single-writer warning"
grep -q 'vector.*dimension\|dimension.*vector' docs/skills/migration.md && echo "✓ vector dimension gotcha"
grep -q 'debugging.md' docs/skills/migration.md && echo "✓ exit to debugging.md"
grep -q 'docs/import.md\|import\.md' docs/skills/migration.md && echo "✓ link to import.md"
```

---

- [ ] **Step 3.4: Write `docs/skills/migration.md`**

Fill in the migration block syntax from Step 3.1 (verify against `docs/HQL.md`).
Fill in import flags from Step 3.2 (verify against source).

```markdown
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
version to the next:

```hql
schema::1 {
    N::User {
        name:  String,
        email: String,
    }
}

MIGRATION schema::1 => schema::2 {
    Node User {
        name  => full_name            # rename field
        email => email                # identity (no change)
        # new required field with default:
        status => status = "active"   # literal default
        # timestamp field:
        created_at => created_at = NOW
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

**Supported field transforms:**

| Transform | Syntax | Notes |
|-----------|--------|-------|
| Rename | `old_name => new_name` | Field value is preserved |
| Type cast | `count: I32 => count: I64` | Must be a safe widening cast |
| Literal default | `field => field = "value"` | Applied to all existing nodes |
| Timestamp default | `field => field = NOW` | Sets current UTC time on all existing nodes |
| Identity | `field => field` | No change; explicit is clearer |

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
cargo run -p sparrow-cli -- check --features lmdb,server
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

---

### Step 8 — Run the import

```bash
sparrow import users.csv     --query CreateUser
sparrow import products.json --query CreateProduct
sparrow import events.parquet --query ImportEvent
```

Key flags (verify exact flag names against `sparrow import --help`):
- `--workers N` — parallel worker count (default 8)
- `--batch-size N` — rows per batch
- `--dry-run` — validate mapping without writing data
- `--token <api-key>` — auth token if instance requires it

Supported formats: **CSV**, **JSON** (array of objects), **Parquet**

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

---

## Common Failure Modes

| Symptom | Cause | Fix |
|---------|-------|-----|
| Compile error on migration block | Version gap or syntax error | Read ariadne output; ensure contiguous version numbers |
| `VECTOR_ERROR` after migration | Vector dimension changed | Drop the old vector type, re-embed into a fresh `vector(N)` type |
| `GRAPH_ERROR` on import | `AddN` duplicate ID | Switch to `UpsertN` in the import query |
| Import rows silently skipped | Type mismatch in CSV column | Run with `--dry-run` first; check column names match query params exactly |
| Restore fails | Snapshot was taken while write txn was open | Take snapshots between operations, not during heavy write load |

---

*Full HQL migration syntax → `docs/HQL.md`*
*Import format details → `docs/import.md`*
*Something broke → `docs/skills/debugging.md`*
```

---

- [ ] **Step 3.5: Run the validation checklist from Step 3.3**

```bash
grep -q 'skill: migration' docs/skills/migration.md && echo "✓ frontmatter slug"
grep -q 'type: workflow' docs/skills/migration.md && echo "✓ frontmatter type"
grep -q 'entry_point:' docs/skills/migration.md && echo "✓ entry_point"
grep -q 'sparrow data snapshot' docs/skills/migration.md && echo "✓ snapshot command"
grep -q 'MIGRATION schema' docs/skills/migration.md && echo "✓ migration block syntax"
grep -q 'sparrow check' docs/skills/migration.md && echo "✓ check before deploy"
grep -q 'sparrow push' docs/skills/migration.md && echo "✓ deploy command"
grep -q 'sparrow import' docs/skills/migration.md && echo "✓ import command"
grep -q 'single.writer\|single writer' docs/skills/migration.md && echo "✓ single-writer warning"
grep -q 'vector.*dimension\|dimension.*vector' docs/skills/migration.md && echo "✓ vector dimension gotcha"
grep -q 'debugging.md' docs/skills/migration.md && echo "✓ exit to debugging.md"
grep -q 'docs/import.md\|import\.md' docs/skills/migration.md && echo "✓ link to import.md"
```

Expected: all 12 lines print `✓`. Fix any that fail.

---

- [ ] **Step 3.6: Commit**

```bash
git add docs/skills/migration.md
git commit -m "docs(skills): add migration workflow skill"
```

---

## Task 4: Write `docs/skills/debugging.md`

**Files:**
- Create: `docs/skills/debugging.md`

**Sources to read before writing:**
- `CLAUDE.md` — workspace-level invariants (tokio::process::Command, LMDB single-writer)
- `docs/HTTP_API.md` — error codes and endpoint list
- `crates/sparrow-core/Cargo.toml` — feature flags
- `crates/sparrow-container/src/main.rs` — `SPARROW_RUNTIME_HQL` env var

---

- [ ] **Step 4.1: Verify the runtime eval env var name**

```bash
grep -n 'RUNTIME' crates/sparrow-container/src/main.rs
```

Confirm the exact env var name. The plan uses `SPARROW_RUNTIME_HQL` — update if source differs.

---

- [ ] **Step 4.2: Verify dev-instance endpoints**

```bash
grep -rn 'node_details\|nodes_by_label\|node_connections' crates/sparrow-core/src/ | head -10
```

Confirm these endpoints exist and note any name changes.

---

- [ ] **Step 4.3: Write the validation checklist**

```bash
grep -q 'skill: debugging' docs/skills/debugging.md && echo "✓ frontmatter slug"
grep -q 'type: workflow' docs/skills/debugging.md && echo "✓ frontmatter type"
grep -q 'entry_point:' docs/skills/debugging.md && echo "✓ entry_point"
grep -q '/diagnostics' docs/skills/debugging.md && echo "✓ diagnostics endpoint"
grep -q '/introspect' docs/skills/debugging.md && echo "✓ introspect endpoint"
grep -q 'SPARROW_RUNTIME_HQL\|__hql_runtime_eval' docs/skills/debugging.md && echo "✓ runtime eval"
grep -q 'INVALID_API_KEY' docs/skills/debugging.md && echo "✓ error code INVALID_API_KEY"
grep -q 'GRAPH_ERROR' docs/skills/debugging.md && echo "✓ error code GRAPH_ERROR"
grep -q 'VECTOR_ERROR' docs/skills/debugging.md && echo "✓ error code VECTOR_ERROR"
grep -q 'tokio::process' docs/skills/debugging.md && echo "✓ async hang / tokio fix"
grep -q 'single.writer\|WorkerPool' docs/skills/debugging.md && echo "✓ LMDB single-writer"
grep -q 'sparrow logs' docs/skills/debugging.md && echo "✓ log streaming"
grep -q 'sparrow stress' docs/skills/debugging.md && echo "✓ stress command"
grep -q 'soft.delet' docs/skills/debugging.md && echo "✓ HNSW soft-delete"
grep -q 'serial_test\|test-threads' docs/skills/debugging.md && echo "✓ serial test note"
```

---

- [ ] **Step 4.4: Write `docs/skills/debugging.md`**

Use the env var name confirmed in Step 4.1. Verify endpoint names from Step 4.2.

```markdown
---
skill: debugging
type: workflow
trigger: >
  Use when a SparrowDB instance, query, or build is behaving
  unexpectedly — compile errors, HTTP error responses, wrong query
  results, performance issues, or async hangs.
entry_point: "Step 1 — Classify the symptom"
exits:
  - setup.md      # if the instance is not running at all
  - migration.md  # if a schema change caused the regression
related:
  - docs/HTTP_API.md
  - docs/auth.md
  - CLAUDE.md
---

# SparrowDB — Debugging Workflow

---

## Step 1 — Classify the symptom

```
A — Compile / build error
    HQL compiler error (ariadne output), Cargo build failure

B — Runtime HTTP error
    HTTP response with non-2xx status and a JSON "code" field

C — Wrong results
    Query returns unexpected data, missing nodes, wrong shape

D — Performance
    Slow queries, high latency, timeouts under load

E — Async hang / deadlock
    Process stops responding; Tokio runtime appears blocked
```

---

## Step 2 — Run baseline checks

Always run these first regardless of symptom:

```bash
# Is the instance healthy?
curl -H "x-api-key: $TOKEN" http://localhost:6969/diagnostics

# Does the schema look right?
curl -H "x-api-key: $TOKEN" http://localhost:6969/introspect
```

`/diagnostics` returns:
```json
{
  "nodes": <count>,
  "edges": <count>,
  "vectors": {
    "total": <count>,
    "active": <count>,
    "soft_deleted": <count>,
    "hnsw_edges": <count>,
    "entry_point_present": <bool>
  }
}
```

Note: high `soft_deleted` count indicates HNSW index degradation (see **Symptom C / D**).

---

## Step 3 — Isolate with runtime eval

Enable the dynamic eval endpoint to test a query without a compiled endpoint:

```bash
# Start instance with runtime eval enabled
SPARROW_RUNTIME_HQL=1 sparrow run

# Send a raw HQL statement
curl -X POST http://localhost:6969/__hql_runtime_eval \
     -H "Content-Type: application/json" \
     -H "x-api-key: $TOKEN" \
     -d '{"query": "N<User>(\"some-id\") RETURN _"}'
```

Use this to reproduce issues with the smallest possible query before looking at
complex multi-step queries.

---

## Step 4 — Branch on symptom

### Symptom A — Compile / build error

```
1. Read the ariadne error output carefully:
     → file path : line : column with underline and note
     → the note tells you what the compiler expected

2. Check feature flags — tests need both storage and server:
     cargo test --features lmdb,server

3. If the compiler feature itself fails to compile:
     → ensure the ariadne crate is present in sparrow-core/Cargo.toml
     → ariadne MUST be included when the `compiler` feature is active

4. Grammar / syntax error in .hx file:
     → match your syntax against docs/HQL.md
     → the PEG grammar lives at crates/sparrow-core/src/grammar.pest
```

---

### Symptom B — Runtime HTTP error

| Error code | HTTP | Cause | Fix |
|------------|------|-------|-----|
| `INVALID_API_KEY` | 401 | Missing or wrong `x-api-key` header | Check header; see `docs/auth.md` |
| `FORBIDDEN` | 403 | Token role is too low | Use `admin` or `read_write` role token |
| `NOT_FOUND` (query) | 404 | Query name not registered | Check `/introspect` for registered route names; case-sensitive |
| `NOT_FOUND` (v1) | 404 | `/v1/query` traffic hitting wildcard handler | The `/v1/query` route MUST be registered before the `/{*path}` wildcard |
| `GRAPH_ERROR` | 500 | Storage-level failure | Check if `write_txn()` was called outside the WorkerPool writer thread |
| `VECTOR_ERROR` | 500 | HNSW / embedding failure | Check vector dimension mismatch; check `soft_deleted` count |

---

### Symptom C — Wrong results

```
Missing or wrong nodes:
  → Check WHERE predicate logic — AND binds tighter than OR
  → Check edge direction — Out<E> vs In<E>; FromN vs ToN

Wrong return shape:
  → Check field remapping — !{fields} excludes from response only,
    not from storage; spread .. may be including unexpected fields

Stale vector results (ghost neighbours in similarity search):
  → DROP on a node soft-deletes its HNSW entry but does NOT compact
  → Stale entries accumulate and degrade recall over time
  → Check: high `soft_deleted` in /diagnostics confirms this
  → Fix: re-embed the collection into a fresh vector type
         (no in-place compaction is currently available)

Result count mismatch:
  → Check RANGE/FIRST operators — may be slicing the result set
  → Check GROUP_BY — changes the shape of results
```

---

### Symptom D — Performance

```
1. Run a load test to measure baseline:
     sparrow stress <instance>

2. Check /diagnostics for HNSW health:
     → high soft_deleted / total ratio → index degraded
     → entry_point_present = false → HNSW is empty or corrupted

3. Write throughput bottleneck:
     → All writes serialise through the single WorkerPool writer
     → Batch writes where possible (BatchAddV for vectors)
     → Consider whether RocksDB backend fits your workload better
       (check crates/sparrow-core/Cargo.toml for current backend options)

4. Slow queries:
     → Add WHERE filters early in the traversal chain to prune the graph
     → Use indexed fields (INDEX, UNIQUE INDEX) in WHERE predicates
     → Vector search k value: smaller k = faster but less recall
```

---

### Symptom E — Async hang / deadlock

**Most common cause: `std::process::Command` inside an async function.**

```
# Wrong — blocks the Tokio thread pool:
let output = std::process::Command::new("docker").status()?;

# Correct:
let output = tokio::process::Command::new("docker").status().await?;
```

Search for the violation:
```bash
grep -rn 'std::process::Command' crates/ --include='*.rs'
```

Any hit inside an `async fn` is a bug. Replace with `tokio::process::Command`.

**Second most common: write transaction held across an await point.**
LMDB write locks must not cross `.await` boundaries. A `write_txn()` must be
acquired, used, and committed/aborted within a single synchronous block.

```bash
# Enable Tokio tracing to find blocked tasks:
RUST_LOG=tokio=trace sparrow run 2>&1 | grep -i 'block\|park\|poll'
```

---

## Enabling Debug Output

Build sparrow-core with the `debug-output` feature for verbose macro expansion
diagnostics (prints generated Rust code during compilation):

```bash
cargo build -p sparrow-core --features lmdb,server,debug-output
```

Enable runtime logging:
```bash
RUST_LOG=sparrow_db=debug sparrow run
```

---

## Log Streaming

Stream logs from a running Docker instance:

```bash
sparrow logs <instance>
```

Example:
```bash
sparrow logs dev
```

---

## Dev-Only Debug Endpoints

Available when the instance is built with the `dev-instance` feature flag:

```bash
# Fetch a specific node by ID
curl -X POST -H "x-api-key: $TOKEN" \
     -d '{"id": "<node-id>"}' \
     http://localhost:6969/node_details

# List all nodes of a type
curl -X POST -H "x-api-key: $TOKEN" \
     -d '{"label": "User"}' \
     http://localhost:6969/nodes_by_label

# Get edges and neighbours of a node
curl -X POST -H "x-api-key: $TOKEN" \
     -d '{"id": "<node-id>"}' \
     http://localhost:6969/node_connections
```

These endpoints are not available in production builds (`production` feature).

---

## Known HNSW Caveats

- **Soft delete accumulation**: `DROP` on a node marks its HNSW vector as inactive
  but does not remove its graph edges from the index. Over time, deleted entries
  degrade recall precision. Monitor `soft_deleted` in `/diagnostics`.
- **No hard delete / compaction**: currently unavailable. Mitigation: re-embed the
  collection into a fresh vector type after heavy deletion.
- **`entry_point_present: false`**: the HNSW graph has no entry point — the vector
  collection is empty or was never populated.

---

## Test Isolation

LMDB stress tests must be run with a single thread to avoid write transaction
conflicts:

```bash
cargo test --package sparrow-core --features lmdb -- --test-threads=1
```

Tests marked with `#[serial]` (from the `serial_test` crate) enforce this
automatically when run through the normal test harness with `--test-threads=1`.

---

*HTTP error codes → `docs/HTTP_API.md`*
*Auth flow → `docs/auth.md`*
*Setup from scratch → `docs/skills/setup.md`*
*Schema migrations → `docs/skills/migration.md`*
```

---

- [ ] **Step 4.5: Run the validation checklist from Step 4.3**

```bash
grep -q 'skill: debugging' docs/skills/debugging.md && echo "✓ frontmatter slug"
grep -q 'type: workflow' docs/skills/debugging.md && echo "✓ frontmatter type"
grep -q 'entry_point:' docs/skills/debugging.md && echo "✓ entry_point"
grep -q '/diagnostics' docs/skills/debugging.md && echo "✓ diagnostics endpoint"
grep -q '/introspect' docs/skills/debugging.md && echo "✓ introspect endpoint"
grep -q 'SPARROW_RUNTIME_HQL\|__hql_runtime_eval' docs/skills/debugging.md && echo "✓ runtime eval"
grep -q 'INVALID_API_KEY' docs/skills/debugging.md && echo "✓ error code INVALID_API_KEY"
grep -q 'GRAPH_ERROR' docs/skills/debugging.md && echo "✓ error code GRAPH_ERROR"
grep -q 'VECTOR_ERROR' docs/skills/debugging.md && echo "✓ error code VECTOR_ERROR"
grep -q 'tokio::process' docs/skills/debugging.md && echo "✓ async hang / tokio fix"
grep -q 'single.writer\|WorkerPool' docs/skills/debugging.md && echo "✓ LMDB single-writer"
grep -q 'sparrow logs' docs/skills/debugging.md && echo "✓ log streaming"
grep -q 'sparrow stress' docs/skills/debugging.md && echo "✓ stress command"
grep -q 'soft.delet' docs/skills/debugging.md && echo "✓ HNSW soft-delete"
grep -q 'serial_test\|test-threads' docs/skills/debugging.md && echo "✓ serial test note"
```

Expected: all 15 lines print `✓`. Fix any that fail.

---

- [ ] **Step 4.6: Commit**

```bash
git add docs/skills/debugging.md
git commit -m "docs(skills): add debugging workflow skill"
```

---

## Task 5: Final cross-reference and index pass

**Files:**
- Verify: all four `docs/skills/*.md` files
- No new files

---

- [ ] **Step 5.1: Verify all cross-references resolve**

```bash
# Each skill should link to the others where appropriate
grep -l 'querying.md'  docs/skills/*.md
grep -l 'setup.md'     docs/skills/*.md
grep -l 'migration.md' docs/skills/*.md
grep -l 'debugging.md' docs/skills/*.md

# Cross-references to source docs should exist
grep -rn 'docs/HQL.md\|HTTP_API.md\|auth.md\|import.md' docs/skills/
```

Expected: no dead paths. All linked files exist under `docs/`.

---

- [ ] **Step 5.2: Check for any placeholder text that leaked through**

```bash
grep -rni 'TBD\|TODO\|FIXME\|placeholder\|fill in\|verify exact' docs/skills/
```

Expected: no output. If any lines appear, fix them before committing.

---

- [ ] **Step 5.3: Commit the final pass (if any fixes were made)**

```bash
git add docs/skills/
git commit -m "docs(skills): cross-reference and placeholder cleanup pass"
```

If no changes were needed, skip this commit.

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Covered by |
|-----------------|-----------|
| Type C reference for querying | `querying.md` Task 1 |
| Concept map, query anatomy | `querying.md` §Concept Map, §Query Anatomy |
| Pattern library (10 patterns) | `querying.md` §Pattern Library |
| `#[mcp]` and `#[model]` annotations | `querying.md` §MCP Tool Exposure |
| Type system table | `querying.md` §Type System |
| Gotchas (ID, dims, soft-delete, UpsertN, remapping) | `querying.md` §Gotchas |
| Operator quick-reference table | `querying.md` §Operator Quick Reference |
| Type A workflow for setup | `setup.md` Task 2 |
| Chef fast path | `setup.md` §Step 1a |
| CLI manual path | `setup.md` §Step 1b–4 |
| Auth token seeding (link, not duplicate) | `setup.md` §Step 5 |
| Verify with /introspect + /diagnostics | `setup.md` §Step 6 |
| Env var table | `setup.md` §Environment Variables |
| Feature flag cheat-sheet | `setup.md` §Feature Flags |
| Common failure modes | `setup.md` §Common Failure Modes |
| Type A workflow for migration | `migration.md` Task 3 |
| Snapshot before migrating | `migration.md` §Step 2 |
| Migration block syntax | `migration.md` §Step 3 |
| Field transform reference table | `migration.md` §Step 3 |
| Validate with `sparrow check` | `migration.md` §Step 4 |
| Bulk import (CSV/JSON/Parquet) | `migration.md` §Steps 7–9 |
| LMDB single-writer warning | `migration.md` §Single-Writer Invariant |
| Migration failure modes | `migration.md` §Common Failure Modes |
| Type A workflow for debugging | `debugging.md` Task 4 |
| Symptom classification (A–E) | `debugging.md` §Step 1 |
| Baseline checks (/diagnostics, /introspect) | `debugging.md` §Step 2 |
| Runtime eval isolation | `debugging.md` §Step 3 |
| Decision tree per symptom | `debugging.md` §Step 4 (A–E) |
| Debug-output feature flag | `debugging.md` §Enabling Debug Output |
| `sparrow logs` streaming | `debugging.md` §Log Streaming |
| Dev-only endpoints | `debugging.md` §Dev-Only Debug Endpoints |
| HNSW soft-delete caveats | `debugging.md` §Known HNSW Caveats |
| Serial test requirement | `debugging.md` §Test Isolation |
| Non-goals (no SDK docs, no "migrate away") | Enforced by omission; links point outward |

**No gaps found.**
