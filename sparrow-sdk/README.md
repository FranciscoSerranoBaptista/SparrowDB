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
            .out(Some("FOLLOWS"))
            .dedup()
            .limit(100),
    )
    .returning(["user", "friends"]);
```

```rust
read_batch()
    .var_as(
        "active_users",
        g()
            .n_with_label_where("User", SourcePredicate::eq("status", "active"))
            .where_(Predicate::gt("score", 100i64))
            .order_by("score", Order::Desc)
            .limit(25)
            .value_map(Some(vec!["$id", "name", "score"])),
    )
    .returning(["active_users"]);
```

```rust
let statuses = Expr::param("statuses");

read_batch()
    .var_as(
        "matching_users",
        g()
            .n_with_label("User")
            .where_(Predicate::is_in_expr("status", statuses))
            .value_map(Some(vec!["$id", "name", "status"])),
    )
    .returning(["matching_users"]);
```

## Conditional Queries

Use `BatchCondition` with `var_as_if` to run later queries only when earlier variables satisfy runtime conditions.

```rust
read_batch()
    .var_as(
        "user",
        g().n_where(SourcePredicate::eq("username", "alice")),
    )
    .var_as_if(
        "posts",
        BatchCondition::VarNotEmpty("user".to_string()),
        g().n(NodeRef::var("user")).out(Some("POSTED")),
    )
    .returning(["user", "posts"]);
```

## Write Batches

```rust
write_batch()
    .var_as(
        "alice",
        g().add_n("User", vec![("name", "Alice"), ("tier", "pro")]),
    )
    .var_as("bob", g().add_n("User", vec![("name", "Bob")]))
    .var_as(
        "linked",
        g()
            .n(NodeRef::var("alice"))
            .add_e(
                "FOLLOWS",
                NodeRef::var("bob"),
                vec![("since", "2026-01-01")],
            )
            .count(),
    )
    .returning(["alice", "bob", "linked"]);
```

```rust
write_batch()
    .var_as(
        "inactive_users",
        g().n_with_label_where(
            "User",
            SourcePredicate::eq("status", "inactive"),
        ),
    )
    .var_as_if(
        "deactivated_count",
        BatchCondition::VarNotEmpty("inactive_users".to_string()),
        g()
            .n(NodeRef::var("inactive_users"))
            .set_property("deactivated", true)
            .count(),
    )
    .returning(["deactivated_count"]);
```

## Executing Queries with `helix_db::Client`

`helix_db::Client` is a thin async wrapper over `reqwest` for running queries against a Helix
instance. Construct it with an optional base URL, then optionally attach a bearer API key:

```rust
use helix_db::Client;

// Defaults to http://localhost:6969 when `url` is None.
let client = Client::new(None)?;

// Or point at a remote cluster and attach an API key:
let client = Client::new(Some("https://11e2fc88c410fa5eb13e.cluster.helix-db.com"))?
    .with_api_key(Some("hx_your_api_key"));
```

Requests are built with a small fluent builder. Start with `client.query::<R>()` (where `R` is
the type you want the response deserialized into), optionally toggle request headers, then choose
a query kind and `.send().await`:

```rust
// Inline / dynamic query: POSTs a `DynamicQueryRequest` (DSL query + parameters) to `/v1/query`.
let response: MyResponse = client
    .query()
    .dynamic_query(request)        // `request` is a DynamicQueryRequest (see below)
    .send()
    .await?;

// Stored query: POSTs a serializable payload to a deployed query's route
// (`/v1/query/<name>`, e.g. `/v1/query/add_user`).
let response: MyResponse = client
    .query()
    .body(&payload)?               // optional request body for the route
    .stored_query("add_user".to_string())
    .send()
    .await?;
```

Optional header toggles can be chained before choosing the query kind:

- `.writer_only()` — require the request to be served by a writer node (`x-helix-require-writer`).
- `.warm_only()` — only execute if the query is already warm (`x-helix-warm`); reads only.
- `.should_await_durability(true)` — block until the write is durable (`x-helix-await-durable`).

`send()` is generic over the deserialized response type `R` and returns `Result<R, HelixError>`.
`HelixError` distinguishes transport errors, non-200 responses from the server (`RemoteError`),
serialization failures, and invalid URLs.

### Registered queries + `dynamic_query`

Annotate a query builder with `#[register]` to get a callable helper that builds a
`DynamicQueryRequest` directly from typed arguments. The generated function returns the request
value itself (not a `Result`) — parameter coercion that can fail (e.g. `DateTime`, bytes) panics
with a descriptive message rather than returning an error.

```rust
use helix_db::dsl::prelude::*;
use helix_db::Client;
use serde::Deserialize;

#[register]
pub fn add_user(name: String) -> WriteBatch {
    write_batch()
        .var_as("user_id", g().add_n("user", vec![("name", name)]))
        .returning(vec!["user_id"])
}

#[derive(Deserialize)]
struct AddUserResponse {
    user_id: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Some("https://11e2fc88c410fa5eb13e.cluster.helix-db.com"))?
        .with_api_key(Some("hx_your_api_key"));

    // Building the request is infallible — no `?` needed here.
    let request = add_user("John".to_string());

    let response: AddUserResponse = client.query().dynamic_query(request).send().await?;
    println!("created user {}", response.user_id);
    Ok(())
}
```

Notes:
- A `#[register]` builder generates a public callable helper only when the function is `pub`.
- The serialized payload includes `request_type`, `query`, and optional `parameters` /
  `parameter_types`.
- Private `#[register]` functions are still registered for bundle generation
  (`helix_db::query_generator::generate()`), but they do not generate the public callable helper.

## Vector Search Operations (End-to-End)

The current Helix interpreter executes vector search as top-k nearest-neighbor lookup with these runtime semantics:
- returns up to `k` hits (top-k behavior)
- hit order is ascending by `$distance` (smaller is closer)
- hit metadata can be read through virtual fields in projections:
  - node hits: `$id`, `$distance`
  - edge hits: `$id`, `$from`, `$to`, `$distance`

### Result field contract

| Field | Type | Node hits | Edge hits | Meaning |
|---|---|---:|---:|---|
| `$id` | integer | yes | yes* | Node ID (for node hits) or edge ID (for edge hits) |
| `$distance` | floating-point | yes | yes | Vector distance from query (`lower` = closer) |
| `$from` | integer | no | yes | Edge source node ID |
| `$to` | integer | no | yes | Edge target node ID |

`*` For edge hits, `$id` is present when an edge ID is available in storage.

Contract scope in the current Helix interpreter:
- available on direct vector-hit streams and projection terminals
- available in `value_map`, `values`, `project`, and (for edges) `edge_properties`
- once a traversal step leaves the hit stream (`out`, `in_`, `both`, etc.), downstream traversers no longer carry distance metadata

### 1) Create indexes and insert vectors

```rust
write_batch()
    .var_as(
        "create_doc_index",
        g().create_vector_index_nodes(
            "Doc",
            "embedding",
            None::<&str>,
        ),
    )
    .var_as(
        "create_similar_index",
        g().create_vector_index_edges(
            "SIMILAR",
            "embedding",
            None::<&str>,
        ),
    )
    .var_as(
        "doc_a",
        g().add_n(
            "Doc",
            vec![
                ("title", PropertyValue::from("A")),
                ("embedding", PropertyValue::from(vec![1.0f32, 0.0, 0.0])),
            ],
        ),
    )
    .var_as(
        "doc_b",
        g().add_n(
            "Doc",
            vec![
                ("title", PropertyValue::from("B")),
                ("embedding", PropertyValue::from(vec![0.9f32, 0.1, 0.0])),
            ],
        ),
    )
    .returning(["create_doc_index", "doc_a", "doc_b"]);
```

### 2) Node vector search: get ranked hits and fetch node properties

```rust
read_batch()
    .var_as(
        "doc_hits",
        g().vector_search_nodes("Doc", "embedding", vec![1.0f32, 0.0, 0.0], 5, None)
            .value_map(Some(vec!["$id", "$distance", "title"])),
    )
    .returning(["doc_hits"]);
```

```text
doc_hits rows (example shape):
[
  { "$id": 42, "$distance": 0.0031, "title": "A" },
  { "$id": 77, "$distance": 0.0198, "title": "B" }
]
```

### 3) Use `project(...)` on vector hits (including distance)

```rust
read_batch()
    .var_as(
        "ranked_docs",
        g().vector_search_nodes("Doc", "embedding", vec![1.0f32, 0.0, 0.0], 10, None)
            .project(vec![
                PropertyProjection::renamed("$id", "doc_id"),
                PropertyProjection::renamed("$distance", "score"),
                PropertyProjection::new("title"),
            ]),
    )
    .returning(["ranked_docs"]);
```

### 4) Traverse from hit IDs to related entities

Store hit rows (with `$id` + `$distance`) and then use `NodeRef::var(...)` to continue graph traversal from those hit IDs.

```rust
read_batch()
    .var_as(
        "doc_hit_rows",
        g().vector_search_nodes("Doc", "embedding", vec![1.0f32, 0.0, 0.0], 5, None)
            .value_map(Some(vec!["$id", "$distance", "title"])),
    )
    .var_as(
        "authors",
        g().n(NodeRef::var("doc_hit_rows"))
            .out(Some("AUTHORED_BY"))
            .value_map(Some(vec!["$id", "name"])),
    )
    .returning(["doc_hit_rows", "authors"]);
```

### 5) Edge vector search and endpoint/property extraction

```rust
read_batch()
    .var_as(
        "edge_hits",
        g().vector_search_edges("SIMILAR", "embedding", vec![1.0f32, 0.0, 0.0], 10, None)
            .edge_properties(),
    )
    .var_as(
        "targets",
        g().e(EdgeRef::var("edge_hits"))
            .out_n()
            .value_map(Some(vec!["$id", "title"])),
    )
    .returning(["edge_hits", "targets"]);
```

`edge_hits` rows include `$from`, `$to`, and `$distance` (and `$id` when available), so you can inspect ranking metadata and still traverse from those edges.

### 6) Optional multitenancy

```rust
write_batch()
    .var_as(
        "create_mt_index",
        g().create_vector_index_nodes(
            "Doc",
            "embedding",
            Some("tenant_id"),
        ),
    )
    .var_as(
        "insert_acme",
        g().add_n(
            "Doc",
            vec![
                ("tenant_id", PropertyValue::from("acme")),
                ("title", PropertyValue::from("Acme doc")),
                ("embedding", PropertyValue::from(vec![1.0f32, 0.0, 0.0])),
            ],
        ),
    )
    .returning(["create_mt_index", "insert_acme"]);
```

```rust
read_batch()
    .var_as(
        "acme_hits",
        g().vector_search_nodes(
            "Doc",
            "embedding",
            vec![1.0f32, 0.0, 0.0],
            5,
            Some(PropertyValue::from("acme")),
        )
        .value_map(Some(vec!["$id", "$distance", "title"])),
    )
    .returning(["acme_hits"]);
```

Multitenant behavior in the current Helix interpreter:
- multitenant index + missing `tenant_value` on search => query error
- multitenant index + unknown tenant => empty result set
- write with vector present but missing tenant property => write error

## Edge-First Reads

```rust
read_batch()
    .var_as(
        "heavy_edges",
        g()
            .e_where(SourcePredicate::gt("weight", 0.8f64))
            .edge_has_label("FOLLOWS")
            .order_by("weight", Order::Desc)
            .limit(50),
    )
    .var_as(
        "targets",
        g()
            .e(EdgeRef::var("heavy_edges"))
            .out_n()
            .dedup(),
    )
    .returning(["heavy_edges", "targets"]);
```

## Branching and Repetition

```rust
read_batch()
    .var_as(
        "recommendations",
        g()
            .n(1u64)
            .store("seed")
            .repeat(RepeatConfig::new(sub().out(Some("FOLLOWS"))).times(2))
            .without("seed")
            .union(vec![sub().out(Some("LIKES"))])
            .dedup()
            .limit(30),
    )
    .returning(["recommendations"]);
```

## Traversal Building Inside `var_as(...)`

Common source steps:
- `n(...)`, `n_where(...)`, `n_with_label(...)`
- `e(...)`, `e_where(...)`, `e_with_label(...)`
- `vector_search_nodes(...)`, `vector_search_edges(...)`
  - current Helix runtime exposes vector hit metadata via virtual fields (`$id`, `$distance`, `$from`, `$to`) in terminal projections

Common navigation and filtering:
- `out/in_/both`, `out_e/in_e/both_e`, `out_n/in_n/other_n`
- `has`, `has_label`, `has_key`, `where_`, `within`, `without`, `dedup`
- `limit`, `skip`, `range`, `order_by`, `order_by_multiple`

Common terminal projections:
- `count`, `exists`, `id`, `label`
- `values`, `value_map`, `project`, `edge_properties`

Write-only operations (usable in `write_batch()` traversals):
- `add_n`, `add_e`, `set_property`, `remove_property`, `drop`, `drop_edge`, `drop_edge_by_id`
- `create_vector_index_nodes`, `create_vector_index_edges`

For exhaustive catalog-style coverage of every public query-builder function, read the crate docs in `src/lib.rs` and browse the source directly.

## License

Licensed under Apache-2.0.
