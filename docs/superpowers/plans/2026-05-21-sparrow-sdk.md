# sparrow-sdk Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the upstream HelixDB Rust SDK into `sparrow-sdk/` at the repo root, rebrand all Helix symbols to Sparrow, write a README, and write `sparrowdb-sdk-llm.txt`.

**Architecture:** Copy `sdks/rust/` to `sparrow-sdk/`, rewrite `Cargo.toml`, apply targeted sed renames across source files, write two documentation files, then delete `sdks/`. All existing unit and doc tests must pass unchanged after the rename.

**Tech Stack:** Rust/Cargo, sed (BSD/macOS `-i ''` syntax), `helix-dsl-macros = "0.2.0"` from crates.io (kept as-is — macro implementation detail, not a branding surface).

---

## File Map

| Path | Action |
|------|--------|
| `sparrow-sdk/Cargo.toml` | Create (rewritten from `sdks/rust/Cargo.toml`) |
| `sparrow-sdk/src/lib.rs` | Create (copied + renamed symbols) |
| `sparrow-sdk/src/dsl.rs` | Create (copied + doc comment renames) |
| `sparrow-sdk/src/query_generator.rs` | Create (copied, no changes needed) |
| `sparrow-sdk/README.md` | Create (new) |
| `sparrow-sdk/sparrowdb-sdk-llm.txt` | Create (new) |
| `Cargo.toml` (root) | Modify — add `"sparrow-sdk"` to `members` |
| `sdks/` | Delete after verification |

---

## Task 1: Scaffold `sparrow-sdk/` and fix `Cargo.toml`

**Files:**
- Create: `sparrow-sdk/Cargo.toml`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Copy source tree**

```bash
cp -r sdks/rust sparrow-sdk
```

- [ ] **Step 2: Rewrite `sparrow-sdk/Cargo.toml`**

Replace the entire file with:

```toml
[package]
name = "sparrow-sdk"
version = "1.0.0"
edition = "2021"
rust-version = "1.75"
description = "Rust SDK for SparrowDB — query-builder DSL and async HTTP client"
license = "Apache-2.0"
repository = "https://github.com/YOUR_ORG/SparrowDB"
readme = "README.md"
keywords = ["graph", "database", "sparrowdb", "dsl", "query"]
categories = ["database", "data-structures"]

[lib]
path = "src/lib.rs"

[dependencies]
chrono = "0.4"
serde = { version = "1", features = ["derive"] }
sonic-rs = "0.5.8"
inventory = "0.3.24"
helix-dsl-macros = "0.2.0"
reqwest = { version = "0.13.3" }
tokio = { version = "1", features = ["full"] }
thiserror = "2.0.18"
```

Key changes from upstream:
- `name`: `"helix-db"` → `"sparrow-sdk"`
- `description`: updated to SparrowDB
- `keywords`: `"helixdb"` → `"sparrowdb"`
- `helix-dsl-macros`: dropped `path = "helix-dsl-macros"` (path pointed to missing directory); kept crates.io version

- [ ] **Step 3: Add `sparrow-sdk` to root workspace**

In `Cargo.toml` at the repo root, change:

```toml
members = [
    "sparrow-db",
    "sparrow-container",
    "sparrow-macros",
    "sparrow-cli",
    "hql-tests",
    "metrics",
    "sparrow-memory",
]
```

to:

```toml
members = [
    "sparrow-db",
    "sparrow-container",
    "sparrow-macros",
    "sparrow-cli",
    "hql-tests",
    "metrics",
    "sparrow-memory",
    "sparrow-sdk",
]
```

- [ ] **Step 4: Verify crate is found (expect compile errors — symbols not renamed yet)**

```bash
cd /Users/franciscobaptista/Development/SparrowDB && cargo check -p sparrow-sdk 2>&1 | head -30
```

Expected: errors about `helix_db` not resolving (the `extern crate self as helix_db` alias is wrong for the new crate name). This confirms the scaffolding is wired up correctly.

- [ ] **Step 5: Commit scaffold**

```bash
git add sparrow-sdk/ Cargo.toml
git commit -m "chore: scaffold sparrow-sdk crate (renames pending)"
```

---

## Task 2: Rename Helix symbols in `sparrow-sdk/src/lib.rs`

**Files:**
- Modify: `sparrow-sdk/src/lib.rs`

- [ ] **Step 1: Apply sed renames**

Run each command in order from the repo root:

```bash
# Error type: HelixError -> SparrowError
sed -i '' 's/HelixError/SparrowError/g' sparrow-sdk/src/lib.rs

# Client alias: HelixDBClient -> SparrowDBClient
sed -i '' 's/HelixDBClient/SparrowDBClient/g' sparrow-sdk/src/lib.rs

# extern crate self alias (CRITICAL — must match crate name "sparrow-sdk" -> "sparrow_sdk")
sed -i '' 's/extern crate self as helix_db/extern crate self as sparrow_sdk/g' sparrow-sdk/src/lib.rs

# use helix_db:: -> use sparrow_sdk:: (fixes test imports)
sed -i '' 's/use helix_db::/use sparrow_sdk::/g' sparrow-sdk/src/lib.rs

# doc comments
sed -i '' 's/Helix instance/SparrowDB instance/g' sparrow-sdk/src/lib.rs
sed -i '' 's/helix_db::Client/sparrow_sdk::Client/g' sparrow-sdk/src/lib.rs
sed -i '' 's/helix-db Rust SDK/sparrow-sdk Rust SDK/g' sparrow-sdk/src/lib.rs

# test URL (cosmetic)
sed -i '' 's|https://cluster.helix-db.com|https://cluster.sparrowdb.example.com|g' sparrow-sdk/src/lib.rs
```

- [ ] **Step 2: Verify the critical substitutions landed**

```bash
grep -n "HelixError\|HelixDBClient\|helix_db\|helix-db Rust" sparrow-sdk/src/lib.rs
```

Expected: no output (zero matches).

```bash
grep -n "SparrowError\|SparrowDBClient\|sparrow_sdk\|sparrow-sdk Rust" sparrow-sdk/src/lib.rs | head -20
```

Expected: multiple matches showing the renames took effect.

- [ ] **Step 3: Wire-format header strings must be unchanged**

```bash
grep -n "x-helix-" sparrow-sdk/src/lib.rs
```

Expected: three lines — `x-helix-require-writer`, `x-helix-warm`, `x-helix-await-durable` — preserved as-is (server-defined protocol values).

- [ ] **Step 4: Commit**

```bash
git add sparrow-sdk/src/lib.rs
git commit -m "chore: rename Helix symbols to Sparrow in sparrow-sdk/src/lib.rs"
```

---

## Task 3: Rename Helix doc references in `sparrow-sdk/src/dsl.rs`

**Files:**
- Modify: `sparrow-sdk/src/dsl.rs`

The only changes in this file are doc comment strings — `helix_db::` path references in doc-test blocks, and "Helix" brand mentions in prose. `helix_dsl_macros` is **not** renamed (it stays as the upstream crate on crates.io).

- [ ] **Step 1: Apply sed renames**

```bash
# Fix all doc example imports: use helix_db:: -> use sparrow_sdk::
sed -i '' 's/helix_db::/sparrow_sdk::/g' sparrow-sdk/src/dsl.rs

# Fix module-level doc header line
sed -i '' 's/the `helix-db` crate (imported as `helix_db`)/the `sparrow-sdk` crate (imported as `sparrow_sdk`)/g' sparrow-sdk/src/dsl.rs

# Fix "current Helix interpreter" prose
sed -i '' 's/current Helix interpreter/current SparrowDB runtime/g' sparrow-sdk/src/dsl.rs
sed -i '' 's/current Helix runtime/current SparrowDB runtime/g' sparrow-sdk/src/dsl.rs

# Fix HelixDB brand in prose
sed -i '' 's/HelixDB/SparrowDB/g' sparrow-sdk/src/dsl.rs
```

- [ ] **Step 2: Verify `helix_dsl_macros` import is untouched**

```bash
grep "helix_dsl_macros" sparrow-sdk/src/dsl.rs
```

Expected: exactly one line: `pub use helix_dsl_macros::register;`

- [ ] **Step 3: Verify no remaining helix_db or HelixDB references**

```bash
grep -n "helix_db\|HelixDB\|helix-db crate" sparrow-sdk/src/dsl.rs | head -20
```

Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add sparrow-sdk/src/dsl.rs
git commit -m "chore: rename Helix doc references to SparrowDB in sparrow-sdk/src/dsl.rs"
```

---

## Task 4: Verify build and tests pass

**Files:** none (verification only)

- [ ] **Step 1: Build the crate**

```bash
cd /Users/franciscobaptista/Development/SparrowDB && cargo build -p sparrow-sdk
```

Expected: `Compiling sparrow-sdk v1.0.0` followed by no errors.

If there are errors, they will be symbol-not-found errors from the rename — check which step missed a substitution using `grep -n <pattern> sparrow-sdk/src/*.rs`.

- [ ] **Step 2: Run unit tests**

```bash
cargo test -p sparrow-sdk
```

Expected: all tests pass. The test suite covers: `#[register]` macro, parameter type coercion (bool/i64/f64/f32/DateTime/ParamValue/ParamObject/Vec<String>/BTreeMap/bytes), SourcePredicate JSON round-trips, query AST literal-vs-param JSON, `Client::new` URL handling, header toggle assembly, dynamic/stored query routing.

- [ ] **Step 3: Run doc tests**

```bash
cargo test -p sparrow-sdk --doc
```

Expected: all doc tests pass. These exercise every code block in `dsl.rs` — the sed replacement of `helix_db::` → `sparrow_sdk::` in `# use sparrow_sdk::dsl::prelude::*;` lines is what makes them compile.

- [ ] **Step 4: Commit if not already clean**

```bash
git status
```

No uncommitted changes expected at this point.

---

## Task 5: Write `sparrow-sdk/README.md`

**Files:**
- Create: `sparrow-sdk/README.md`

- [ ] **Step 1: Write the README**

Create `sparrow-sdk/README.md` with this exact content:

````markdown
# sparrow-sdk

The Rust SDK for SparrowDB — a graph + vector database. Pairs a composable query-builder DSL with a small async HTTP client for running those queries against a SparrowDB instance.

[Core shape](#core-shape) | [Client](#executing-queries) | [Registered queries](#registered-queries) | [Vector search](#vector-search) | [Traversal reference](#traversal-reference) | [Error handling](#error-handling)

## Install

```toml
[dependencies]
sparrow-sdk = "1.0.0"
```

Import the DSL prelude in query-writing code:

```rust
use sparrow_sdk::dsl::prelude::*;
```

## Quick Start

```rust
use sparrow_sdk::{Client, dsl::prelude::*};
use serde::Deserialize;

#[derive(Deserialize)]
struct User {
    #[serde(rename = "$id")]
    id: u64,
    name: String,
}

#[derive(Deserialize)]
struct Resp { user: Vec<User> }

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(None)?; // defaults to http://localhost:6969

    let batch = read_batch()
        .var_as("user", g().n_where(SourcePredicate::eq("username", "alice")))
        .returning(["user"]);

    let resp: Resp = client
        .query()
        .dynamic_query(DynamicQueryRequest::read(batch))
        .send()
        .await?;

    println!("found {} users", resp.user.len());
    Ok(())
}
```

## Core Shape

Every query follows one of two patterns:

```
read_batch()  → .var_as(...) / .var_as_if(...) → .returning([...])
write_batch() → .var_as(...) / .var_as_if(...) → .returning([...])
```

Each `.var_as("name", traversal)` names a traversal result. `.returning([...])` selects which names appear in the JSON response. Traversals always start with `g()`.

## Read Batches

Find a node by property:

```rust
read_batch()
    .var_as("user", g().n_where(SourcePredicate::eq("username", "alice")))
    .returning(["user"]);
```

Filter, sort, project:

```rust
read_batch()
    .var_as(
        "top_users",
        g().n_with_label_where("User", SourcePredicate::eq("status", "active"))
            .where_(Predicate::gt("score", 100i64))
            .order_by("score", Order::Desc)
            .limit(25)
            .value_map(Some(vec!["$id", "name", "score"])),
    )
    .returning(["top_users"]);
```

Traverse the graph:

```rust
read_batch()
    .var_as("user", g().n_where(SourcePredicate::eq("username", "alice")))
    .var_as(
        "friends",
        g().n(NodeRef::var("user")).out(Some("FOLLOWS")).dedup().limit(100),
    )
    .returning(["user", "friends"]);
```

Parameterised filter (value resolved at runtime):

```rust
let statuses = Expr::param("statuses");

read_batch()
    .var_as(
        "matching",
        g().n_with_label("User")
            .where_(Predicate::is_in_expr("status", statuses))
            .value_map(Some(vec!["$id", "name", "status"])),
    )
    .returning(["matching"]);
```

## Conditional Queries

Use `var_as_if` to skip later steps when earlier results are empty:

```rust
read_batch()
    .var_as("user", g().n_where(SourcePredicate::eq("username", "alice")))
    .var_as_if(
        "posts",
        BatchCondition::VarNotEmpty("user".to_string()),
        g().n(NodeRef::var("user")).out(Some("POSTED")),
    )
    .returning(["user", "posts"]);
```

`BatchCondition` variants: `VarNotEmpty(name)`, `VarEmpty(name)`, `VarMinSize(name, n)`, `PrevNotEmpty`.

## Write Batches

Create nodes and edges:

```rust
write_batch()
    .var_as("alice", g().add_n("User", vec![("name", "Alice"), ("tier", "pro")]))
    .var_as("bob", g().add_n("User", vec![("name", "Bob")]))
    .var_as(
        "linked",
        g().n(NodeRef::var("alice"))
            .add_e("FOLLOWS", NodeRef::var("bob"), vec![("since", "2026-01-01")])
            .count(),
    )
    .returning(["alice", "bob", "linked"]);
```

Conditional mutation:

```rust
write_batch()
    .var_as(
        "inactive",
        g().n_with_label_where("User", SourcePredicate::eq("status", "inactive")),
    )
    .var_as_if(
        "deactivated",
        BatchCondition::VarNotEmpty("inactive".to_string()),
        g().n(NodeRef::var("inactive")).set_property("deactivated", true).count(),
    )
    .returning(["deactivated"]);
```

## Executing Queries

`sparrow_sdk::Client` is a thin async wrapper over `reqwest`.

```rust
use sparrow_sdk::Client;

// Defaults to http://localhost:6969
let client = Client::new(None)?;

// Remote with API key
let client = Client::new(Some("https://mydb.example.com"))?
    .with_api_key(Some("your_api_key"));
```

Build a request with `.query::<R>()` where `R` is your response type:

```rust
// Dynamic query — POSTs the DSL AST to /v1/query
let response: MyResponse = client
    .query()
    .dynamic_query(DynamicQueryRequest::read(my_batch))
    .send()
    .await?;

// Stored query — POSTs a serializable body to /v1/query/<name>
let response: MyResponse = client
    .query()
    .body(&payload)?
    .stored_query("my_query".to_string())
    .send()
    .await?;
```

Optional header toggles (chain before choosing the query kind):

| Method | Header sent | Effect |
|--------|-------------|--------|
| `.writer_only()` | `x-helix-require-writer: true` | Route to a writer node |
| `.warm_only()` | `x-helix-warm: true` | Skip if query not already warm |
| `.should_await_durability(true)` | `x-helix-await-durable: true` | Block until write is durable |

## Registered Queries

The `#[register]` macro turns a query-builder function into a typed callable that produces a `DynamicQueryRequest` directly from typed arguments:

```rust
use sparrow_sdk::dsl::prelude::*;
use sparrow_sdk::Client;
use serde::Deserialize;

#[register]
pub fn add_user(name: String) -> WriteBatch {
    write_batch()
        .var_as("user_id", g().add_n("User", vec![("name", name)]))
        .returning(["user_id"])
}

#[derive(Deserialize)]
struct AddUserResponse { user_id: u64 }

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(None)?;
    let request = add_user("Alice".to_string()); // DynamicQueryRequest, not Result
    let resp: AddUserResponse = client.query().dynamic_query(request).send().await?;
    println!("created user {}", resp.user_id);
    Ok(())
}
```

Rules:
- `pub fn` generates a callable helper and registers for bundle generation.
- Private `fn` registers for bundle generation only — no public callable helper.
- Parameter coercion failures (e.g. `Vec<u8>` bytes on a dynamic query) panic with a descriptive message.
- Call `sparrow_sdk::query_generator::generate()` in a build script to emit `queries.json` from all registered queries.

## Vector Search

### Create an index and insert vectors

```rust
write_batch()
    .var_as("idx", g().create_vector_index_nodes("Doc", "embedding", None::<&str>))
    .var_as(
        "doc",
        g().add_n("Doc", vec![
            ("title", PropertyValue::from("Hello")),
            ("embedding", PropertyValue::from(vec![1.0f32, 0.0, 0.0])),
        ]),
    )
    .returning(["idx", "doc"]);
```

### Node vector search

```rust
read_batch()
    .var_as(
        "hits",
        g().vector_search_nodes("Doc", "embedding", vec![1.0f32, 0.0, 0.0], 5, None)
            .value_map(Some(vec!["$id", "$distance", "title"])),
    )
    .returning(["hits"]);
```

Hits are ordered by `$distance` ascending (smaller = closer). Virtual fields `$id` and `$distance` are available in terminal projections. Once a traversal step leaves the hit stream (`out`, `in_`, etc.), distance metadata is no longer available downstream.

### Edge vector search

```rust
read_batch()
    .var_as(
        "edges",
        g().vector_search_edges("SIMILAR", "embedding", vec![1.0f32, 0.0, 0.0], 10, None)
            .edge_properties(),
    )
    .var_as(
        "targets",
        g().e(EdgeRef::var("edges")).out_n().value_map(Some(vec!["$id", "title"])),
    )
    .returning(["edges", "targets"]);
```

Edge hit rows include `$from`, `$to`, `$distance`, and `$id` (when available in storage).

### Traverse from vector hits

```rust
read_batch()
    .var_as(
        "hit_rows",
        g().vector_search_nodes("Doc", "embedding", vec![1.0f32, 0.0, 0.0], 5, None)
            .value_map(Some(vec!["$id", "$distance", "title"])),
    )
    .var_as(
        "authors",
        g().n(NodeRef::var("hit_rows")).out(Some("AUTHORED_BY")).value_map(Some(vec!["$id", "name"])),
    )
    .returning(["hit_rows", "authors"]);
```

### Multitenancy

```rust
// Create index with tenant field
write_batch()
    .var_as("idx", g().create_vector_index_nodes("Doc", "embedding", Some("tenant_id")))
    .returning(["idx"]);

// Search scoped to a tenant
read_batch()
    .var_as(
        "hits",
        g().vector_search_nodes(
            "Doc", "embedding", vec![1.0f32, 0.0, 0.0], 5,
            Some(PropertyValue::from("acme")),
        )
        .value_map(Some(vec!["$id", "$distance", "title"])),
    )
    .returning(["hits"]);
```

Gotchas: multitenant index + missing `tenant_value` on search → query error; unknown tenant → empty result; write missing tenant property → write error.

## Edge-First Reads

```rust
read_batch()
    .var_as(
        "heavy",
        g().e_where(SourcePredicate::gt("weight", 0.8f64))
            .edge_has_label("FOLLOWS")
            .order_by("weight", Order::Desc)
            .limit(50),
    )
    .var_as("targets", g().e(EdgeRef::var("heavy")).out_n().dedup())
    .returning(["heavy", "targets"]);
```

## Branching and Repetition

```rust
read_batch()
    .var_as(
        "network",
        g().n(NodeRef::id(42))
            .store("seed")
            .repeat(RepeatConfig::new(sub().out(Some("FOLLOWS"))).times(3))
            .without("seed")
            .union(vec![sub().out(Some("LIKES"))])
            .dedup()
            .limit(200),
    )
    .returning(["network"]);
```

## Traversal Reference

### Sources

| Method | Description |
|--------|-------------|
| `g().n(NodeRef)` | Nodes by ID(s), variable, or all |
| `g().n_where(SourcePredicate)` | Nodes matching a source predicate |
| `g().n_with_label(label)` | Nodes with a specific label |
| `g().n_with_label_where(label, pred)` | Nodes with label + predicate |
| `g().e(EdgeRef)` | Edges by ID(s) or variable |
| `g().e_where(SourcePredicate)` | Edges matching a source predicate |
| `g().e_with_label(label)` | Edges with a specific label |
| `g().e_with_label_where(label, pred)` | Edges with label + predicate |
| `g().vector_search_nodes(label, field, vec, k, tenant?)` | Top-k node vector search |
| `g().vector_search_edges(label, field, vec, k, tenant?)` | Top-k edge vector search |

### Navigation

| Method | Description |
|--------|-------------|
| `.out(label?)` | Traverse outgoing edges → nodes |
| `.in_(label?)` | Traverse incoming edges → nodes |
| `.both(label?)` | Traverse both directions → nodes |
| `.out_e(label?)` | Move to outgoing edge objects |
| `.in_e(label?)` | Move to incoming edge objects |
| `.both_e(label?)` | Move to edge objects in both directions |
| `.out_n()` | Move to target nodes of edges |
| `.in_n()` | Move to source nodes of edges |
| `.other_n()` | Move to the other endpoint of edges |

### Filtering

| Method | Description |
|--------|-------------|
| `.where_(Predicate)` | Filter by property predicate |
| `.has(key, value)` | Filter where property equals value |
| `.has_label(label)` | Filter by label |
| `.has_key(key)` | Filter where property key exists |
| `.edge_has(key, value)` | Edge filter by property value |
| `.edge_has_label(label)` | Edge filter by label |
| `.within(store)` | Keep only elements also in store |
| `.without(store)` | Remove elements also in store |
| `.dedup()` | Remove duplicates |

### Shaping

| Method | Description |
|--------|-------------|
| `.limit(n)` | Keep first n results |
| `.skip(n)` | Skip first n results |
| `.range(start, end)` | Slice results |
| `.order_by(field, Order)` | Sort by one field |
| `.order_by_multiple(vec)` | Sort by multiple fields |

### Terminals (projections)

| Method | Description |
|--------|-------------|
| `.count()` | Return count of results |
| `.exists()` | Return bool |
| `.id()` | Return element IDs |
| `.label()` | Return element labels |
| `.values(fields)` | Return specific property values |
| `.value_map(fields?)` | Return property map (all or specific) |
| `.project(projections)` | Return renamed/projected fields |
| `.edge_properties()` | Return edge property map |

### Write operations (`write_batch()` only)

| Method | Description |
|--------|-------------|
| `.add_n(label, props)` | Create a node |
| `.add_e(label, target, props)` | Create an edge |
| `.set_property(key, value)` | Set a property |
| `.remove_property(key)` | Remove a property |
| `.drop()` | Delete current elements |
| `.drop_edge(NodeRef)` | Delete edges to/from nodes |
| `.drop_edge_by_id(EdgeRef)` | Delete edges by ID |
| `g().create_vector_index_nodes(label, field, tenant?)` | Create HNSW node index |
| `g().create_vector_index_edges(label, field, tenant?)` | Create HNSW edge index |

### Flow control

| Method | Description |
|--------|-------------|
| `.store(name)` | Save current stream to named store |
| `.select(name)` | Resume from named store |
| `.inject(name)` | Pull elements from named store |
| `.as_(name)` | Alias current position |
| `.repeat(RepeatConfig)` | Repeat a sub-traversal |
| `.union(vec![sub()])` | Merge multiple sub-traversals |
| `.choose(pred, sub_t, sub_f?)` | Conditional branch |
| `.coalesce(vec![sub()])` | First non-empty sub-traversal |
| `.optional(sub())` | Sub-traversal or empty |
| `.fold()` / `.unfold()` | Collect into / expand from array |
| `.path()` / `.simple_path()` | Emit traversal path |
| `.group(field)` / `.group_count(field)` | Group by field |
| `.aggregate_by(fn, field)` | Aggregate (Count/Sum/Min/Max/Mean) |

## Error Handling

`send()` returns `Result<R, SparrowError>`.

| Variant | When |
|---------|------|
| `SparrowError::ReqwestError(e)` | HTTP transport failure |
| `SparrowError::RemoteError { details }` | Server returned non-200 |
| `SparrowError::SerializationError(e)` | JSON serialization/deserialization failed |
| `SparrowError::InvalidURL(msg)` | Malformed URL passed to `Client::new` |

## License

Apache-2.0
````

- [ ] **Step 2: Commit**

```bash
git add sparrow-sdk/README.md
git commit -m "docs: add sparrow-sdk README"
```

---

## Task 6: Write `sparrow-sdk/sparrowdb-sdk-llm.txt`

**Files:**
- Create: `sparrow-sdk/sparrowdb-sdk-llm.txt`

- [ ] **Step 1: Write the LLM reference file**

Create `sparrow-sdk/sparrowdb-sdk-llm.txt` with this exact content:

```
# sparrow-sdk — LLM Reference
# Rust SDK for SparrowDB. Crate: sparrow-sdk. Import root: sparrow_sdk.

## INSTALL

[dependencies]
sparrow-sdk = "1.0.0"

use sparrow_sdk::dsl::prelude::*;

---

## KEY TYPES

Client
  sparrow_sdk::Client
  Client::new(url: Option<&str>) -> Result<Self, SparrowError>
    url=None => http://localhost:6969
  client.with_api_key(key: Option<&str>) -> Self
  client.query::<R>() -> QueryBuilder<R>

SparrowDBClient
  Type alias for Client (backwards compat)

SparrowError
  ::ReqwestError(reqwest::Error)      HTTP transport failure
  ::RemoteError { details: String }   Server returned non-200
  ::SerializationError(sonic_rs::Error) JSON error
  ::InvalidURL(String)               Bad URL in Client::new

QueryBuilder<R>  (from client.query::<R>())
  .writer_only()                     -> x-helix-require-writer: true
  .warm_only()                       -> x-helix-warm: true
  .should_await_durability(bool)     -> x-helix-await-durable: true/false
  .body<T: Serialize>(&T) -> Result<Self, SparrowError>
  .dynamic_query(DynamicQueryRequest) -> QueryRequest<R>
  .stored_query(name: String) -> QueryRequest<R>

QueryRequest<R>
  .send().await -> Result<R, SparrowError>

DynamicQueryRequest
  ::read(batch: ReadBatch) -> DynamicQueryRequest
  ::write(batch: WriteBatch) -> DynamicQueryRequest
  Wire shape: { "request_type": "Read"|"Write", "query": <ast>,
                "parameters": {...}?, "parameter_types": {...}? }

ReadBatch   result of read_batch()...returning(...)
WriteBatch  result of write_batch()...returning(...)

PropertyValue
  Null | Bool(bool) | I64(i64) | DateTime(i64 millis) | F64(f64) | F32(f32)
  String(String) | Bytes(Vec<u8>)
  I64Array(Vec<i64>) | F64Array(Vec<f64>) | F32Array(Vec<f32>) | StringArray(Vec<String>)
  Array(Vec<PropertyValue>) | Object(BTreeMap<String,PropertyValue>)
  From impls: &str, String, i64, i32, f64, f32, bool, Vec<u8>, Vec<i64>, Vec<f64>,
              Vec<f32>, Vec<String>, Vec<PropertyValue>, BTreeMap, HashMap

NodeRef
  ::id(u64) | ::ids([u64;N] or Vec<u64>) | ::var(name) | ::param(name) | ::all()
  From<u64>, From<Vec<u64>>, From<[u64;N]>, From<&str>

EdgeRef
  ::id(u64) | ::ids([u64;N] or Vec<u64>) | ::var(name) | ::param(name)
  From<u64>, From<Vec<u64>>, From<[u64;N]>

SourcePredicate  (index-friendly; use in n_where/e_where)
  ::eq(prop, value|Expr) | ::neq | ::gt | ::gte | ::lt | ::lte
  ::between(prop, min|Expr, max|Expr)
  ::has_key(prop) | ::starts_with(prop, prefix)
  ::and(vec![..]) | ::or(vec![..])
  Expr variants: EqExpr/NeqExpr/GtExpr/GteExpr/LtExpr/LteExpr/BetweenExpr

Predicate  (traversal filter; use in .where_())
  Same as SourcePredicate plus:
  ::ends_with(prop, suffix) | ::contains(prop, sub) | ::contains_param(prop, param)
  ::is_in(prop, values) | ::is_in_expr(prop, Expr) | ::is_in_param(prop, param)
  ::is_null(prop) | ::is_not_null(prop)
  ::not(pred) | ::compare(left: Expr, op: CompareOp, right: Expr)
  ::eq_param | ::neq_param | ::gt_param | ::gte_param | ::lt_param | ::lte_param

CompareOp  Eq | Neq | Gt | Gte | Lt | Lte

Expr
  ::prop(name) | ::val(v: impl Into<PropertyValue>) | ::id() | ::param(name)
  ::timestamp() | ::datetime()
  arithmetic: .add(Expr) | .sub(Expr) | .mul(Expr) | .div(Expr) | .modulo(Expr) | .neg()
  ::case(when_then: Vec<(Predicate,Expr)>, else: Option<Expr>)

StreamBound  From<usize/u32/u16/u8/i64/i32/Expr>. Used by limit/skip/range.

PropertyProjection
  ::new(name) | ::renamed(source, alias)

ExprProjection  ::new(alias, expr: Expr)

Projection
  ::property(source, alias) | ::expr(alias, expr)
  From<PropertyProjection>, From<ExprProjection>

Order  Asc | Desc

BatchCondition
  VarNotEmpty(name: String)
  VarEmpty(name: String)
  VarMinSize(name: String, n: usize)
  PrevNotEmpty

RepeatConfig
  ::new(sub: SubTraversal)
  .times(n) | .until(Predicate) | .max_depth(n)
  .emit_all() | .emit_before() | .emit_after() | .emit_if(Predicate)

SubTraversal  sub() -> SubTraversal
  Same navigation/filter/shaping steps as main traversal but no typestate.

AggregateFunction  Count | Sum | Min | Max | Mean

DateTime  (UTC epoch milliseconds)
  ::from_millis(i64) | ::parse_rfc3339(&str) -> Result | .millis() -> i64 | .to_rfc3339() -> Option<String>

ParamValue   = PropertyValue
ParamObject  = BTreeMap<String, PropertyValue>

QueryBundle  (for build scripts)
  build_query_bundle() -> Result<QueryBundle, GenerateError>
  generate() -> Result<PathBuf, GenerateError>  // writes queries.json
  generate_to_path(path) -> Result<PathBuf, GenerateError>

---

## DSL ENTRY POINTS

read_batch()  -> ReadBatch builder (no mutation)
write_batch() -> WriteBatch builder (allows mutations)
g()           -> Traversal<Empty, ReadOnly>  — all traversals start here
sub()         -> SubTraversal               — for union/choose/repeat/coalesce/optional

---

## SOURCE STEPS  (start of g() chain)

g().n(NodeRef)
g().n_where(SourcePredicate)
g().n_with_label(label: &str)
g().n_with_label_where(label: &str, pred: SourcePredicate)
g().e(EdgeRef)
g().e_where(SourcePredicate)
g().e_with_label(label: &str)
g().e_with_label_where(label: &str, pred: SourcePredicate)
g().vector_search_nodes(label, field, vec: Vec<f32>, k: usize, tenant: Option<PropertyValue>)
g().vector_search_edges(label, field, vec: Vec<f32>, k: usize, tenant: Option<PropertyValue>)
g().inject(store_name)          pull elements from named store (write_batch only)
g().drop_edge_by_id(EdgeRef)    delete edges by ID from empty source (write_batch only)

---

## NAVIGATION STEPS

OnNodes → OnNodes:
  .out(label?) | .in_(label?) | .both(label?)

OnNodes → OnEdges:
  .out_e(label?) | .in_e(label?) | .both_e(label?)

OnEdges → OnNodes:
  .out_n() | .in_n() | .other_n()

---

## FILTER STEPS

.where_(Predicate)
.has(key, value)                  // value: bool | i64 | f64 | &str
.has_label(label)
.has_key(key)
.edge_has(key, value)             // OnEdges only
.edge_has_label(label)            // OnEdges only
.within(store_name)
.without(store_name)
.dedup()

---

## SHAPING STEPS

.limit(n: impl Into<StreamBound>)
.skip(n: impl Into<StreamBound>)
.range(start, end: impl Into<StreamBound>)
.order_by(field, Order)
.order_by_multiple(vec![(field, Order)])

---

## TERMINAL PROJECTIONS  (→ Terminal state, no further chaining)

.count()
.exists()
.id()
.label()
.values(vec![field])
.value_map(Option<Vec<field>>)    // None = all fields
.project(vec![PropertyProjection | ExprProjection | Projection])
.edge_properties()                // OnEdges only
// vector hit virtual fields: $id, $distance, $from (edges), $to (edges)
// virtual fields are available ONLY before leaving the hit stream

---

## WRITE STEPS  (write_batch only)

.add_n(label: &str, props: vec![(&str, impl Into<PropertyInput>)])
.add_e(label: &str, target: NodeRef, props: vec![...])
.set_property(key, value: impl Into<PropertyInput>)
.remove_property(key)
.drop()
.drop_edge(target: NodeRef)
.drop_edge_by_id(ids: impl Into<EdgeRef>)
g().create_vector_index_nodes(label, field, tenant_field: Option<&str>)
g().create_vector_index_edges(label, field, tenant_field: Option<&str>)

---

## FLOW CONTROL

.store(name)
.select(name)
.inject(name)
.as_(name)
.repeat(RepeatConfig)
.union(branches: Vec<SubTraversal>)
.choose(pred: Predicate, true_branch: SubTraversal, false_branch: Option<SubTraversal>)
.coalesce(branches: Vec<SubTraversal>)
.optional(sub: SubTraversal)
.fold() | .unfold()
.path() | .simple_path()
.group(field) | .group_count(field)
.aggregate_by(AggregateFunction, field)
.with_sack(initial: PropertyValue) | .sack_set(field) | .sack_add(field) | .sack_get()

---

## #[register] MACRO

use sparrow_sdk::dsl::prelude::*;

#[register]
pub fn my_query(arg: String) -> ReadBatch {   // or WriteBatch
    read_batch()
        .var_as("result", g().n_where(SourcePredicate::eq("field", arg)))
        .returning(["result"])
}
// my_query("value") -> DynamicQueryRequest  (not Result)
// pub fn -> callable helper + bundle registration
// private fn -> bundle registration only (no callable helper)
// Vec<u8> param panics on dynamic call; only safe for stored queries

---

## WIRE PROTOCOL

Dynamic query:  POST /v1/query                  body: DynamicQueryRequest JSON
Stored query:   POST /v1/query/<name>           body: serializable payload
Auth:           Authorization: Bearer <api_key>
Content-Type:   application/json

Header flags:
  x-helix-require-writer: true    writer_only()
  x-helix-warm: true              warm_only()
  x-helix-await-durable: true     should_await_durability(true)

---

## ERROR VARIANTS

SparrowError::ReqwestError(e)        HTTP transport (connection refused, timeout)
SparrowError::RemoteError{details}   Server non-200; details = response body text
SparrowError::SerializationError(e)  JSON serialize/deserialize failure
SparrowError::InvalidURL(msg)        URL parse failure in Client::new

---

## RECIPES

### 1. Find node by property
let req = read_batch()
    .var_as("user", g().n_where(SourcePredicate::eq("email", "alice@example.com")))
    .returning(["user"]);
let resp: Resp = client.query().dynamic_query(DynamicQueryRequest::read(req)).send().await?;

### 2. 1-hop graph traversal
let req = read_batch()
    .var_as("user", g().n_where(SourcePredicate::eq("username", "alice")))
    .var_as("friends", g().n(NodeRef::var("user")).out(Some("FOLLOWS")).dedup().limit(50))
    .returning(["user", "friends"]);

### 3. Conditional query (skip if empty)
let req = read_batch()
    .var_as("user", g().n_where(SourcePredicate::eq("username", "alice")))
    .var_as_if(
        "posts",
        BatchCondition::VarNotEmpty("user".to_string()),
        g().n(NodeRef::var("user")).out(Some("POSTED")),
    )
    .returning(["user", "posts"]);

### 4. Create node + edge
let req = write_batch()
    .var_as("a", g().add_n("User", vec![("name", "Alice")]))
    .var_as("b", g().add_n("User", vec![("name", "Bob")]))
    .var_as("e", g().n(NodeRef::var("a")).add_e("FOLLOWS", NodeRef::var("b"), vec![]))
    .returning(["a", "b"]);

### 5. Update property
let req = write_batch()
    .var_as("u", g().n_where(SourcePredicate::eq("username", "alice")))
    .var_as("r", g().n(NodeRef::var("u")).set_property("active", true))
    .returning(["r"]);

### 6. Delete nodes
let req = write_batch()
    .var_as("old", g().n_with_label_where("Session", SourcePredicate::lt("expires_at", 0i64)))
    .var_as("r", g().n(NodeRef::var("old")).drop())
    .returning(["r"]);

### 7. Vector search (nodes)
let req = read_batch()
    .var_as(
        "hits",
        g().vector_search_nodes("Doc", "embedding", query_vec, 10, None)
            .value_map(Some(vec!["$id", "$distance", "title"])),
    )
    .returning(["hits"]);

### 8. Multitenant vector search
let req = read_batch()
    .var_as(
        "hits",
        g().vector_search_nodes(
            "Doc", "embedding", query_vec, 10,
            Some(PropertyValue::from("acme")),
        )
        .value_map(Some(vec!["$id", "$distance", "title"])),
    )
    .returning(["hits"]);

### 9. Traverse from vector hits
let req = read_batch()
    .var_as(
        "hit_rows",
        g().vector_search_nodes("Doc", "embedding", query_vec, 5, None)
            .value_map(Some(vec!["$id", "$distance"])),
    )
    .var_as(
        "authors",
        g().n(NodeRef::var("hit_rows")).out(Some("AUTHORED_BY"))
            .value_map(None::<Vec<&str>>),
    )
    .returning(["hit_rows", "authors"]);

### 10. Repeat traversal (BFS up to depth 3)
let req = read_batch()
    .var_as(
        "network",
        g().n(NodeRef::id(42))
            .store("seed")
            .repeat(RepeatConfig::new(sub().out(Some("FOLLOWS"))).times(3))
            .without("seed")
            .dedup()
            .limit(200),
    )
    .returning(["network"]);

### 11. Registered query + client
#[register]
pub fn get_user(username: String) -> ReadBatch {
    read_batch()
        .var_as("user", g().n_where(SourcePredicate::eq("username", username)))
        .returning(["user"])
}
// Usage (infallible — no ? needed):
let req = get_user("alice".to_string());
let resp: MyResp = client.query().dynamic_query(req).send().await?;

### 12. Stored query
let resp: MyResp = client
    .query()
    .body(&serde_json::json!({"username": "alice"}))?
    .stored_query("get_user".to_string())
    .send()
    .await?;

---

## GOTCHAS

1. Vec<u8> (bytes) parameters panic on dynamic query calls.
   Only safe for stored queries via query_generator::generate().

2. Vector distance metadata ($id, $distance, $from, $to) is available ONLY on hit
   streams and projection terminals. Leaving the hit stream via .out()/.in_()/etc.
   loses distance metadata downstream — you cannot read $distance after traversal.

3. Multitenant index + missing tenant_value on search  = query error (not empty).
   Multitenant index + unknown tenant                  = empty result (no error).
   Write with vector field but missing tenant property  = write error.

4. DynamicQueryRequest::read() vs ::write() must match the batch type (ReadBatch
   vs WriteBatch). Mismatches cause runtime errors on the server.

5. #[register] callable helpers are only generated for pub fn. Private registered
   functions are available in query bundles but have no callable helper.
```

- [ ] **Step 2: Commit**

```bash
git add sparrow-sdk/sparrowdb-sdk-llm.txt
git commit -m "docs: add sparrowdb-sdk-llm.txt LLM reference"
```

---

## Task 7: Clean up `sdks/` and final verification

**Files:**
- Delete: `sdks/`

- [ ] **Step 1: Final build and test run**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo build -p sparrow-sdk
cargo test -p sparrow-sdk
cargo test -p sparrow-sdk --doc
```

All three commands must succeed with zero errors.

- [ ] **Step 2: Verify no helix branding remains in sparrow-sdk (except wire headers and macro crate)**

```bash
grep -rn "HelixError\|HelixDBClient\|helix_db\b\|helix-db\b\|HelixDB" sparrow-sdk/src/ | grep -v "helix_dsl_macros"
```

Expected: no output.

- [ ] **Step 3: Remove `sdks/`**

```bash
rm -rf sdks/
```

- [ ] **Step 4: Confirm workspace still builds cleanly**

```bash
cargo build -p sparrow-sdk
```

Expected: clean build (no references to `sdks/` anywhere in the workspace).

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "chore: remove sdks/ — superseded by sparrow-sdk/"
```
