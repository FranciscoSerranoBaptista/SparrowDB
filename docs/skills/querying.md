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

# HQL Querying Reference

SparrowDB is a graph-vector database. You define schema and queries in `.hx` files using HQL (Hypergraph Query Language). Each compiled `QUERY` becomes a POST endpoint at `/<QueryName>` on the HTTP gateway (default port 6969).

---

## Concept map

| Concept | Syntax | Description |
|---------|--------|-------------|
| Node | `N<Type>` | Labelled vertex with typed fields |
| Edge | `E<Type>` | Directed, typed connection between two nodes |
| Vector | `V<Type>` | Embedding + optional metadata, lives in HNSW index |
| Node-field vector | `vector(N)` | Fixed-dimension embedding field declared directly on a node type |
| Step separator | `::` | Chains operations — traversal, filter, remap, aggregation |
| Current element | `_` | Anonymous reference to the element being iterated inside `WHERE` and remapping |

The `::` operator is HQL's pipeline operator. It has no relationship to Rust's path separator. Every query is a sequence of variable assignments terminated by `RETURN`.

```hql
// Reading left to right: start at User(id), traverse Follows edges,
// filter adults, paginate
result <- N<User>(id)::Out<Follows>::WHERE(_::{age}::GT(18))::RANGE(0, 10)
```

---

## Query anatomy

```hql
QUERY queryName (param1: Type, param2: Type) =>
    // body — variable assignments, top to bottom
    variable <- expression
    RETURN variable
```

- **Parameters** are typed and passed as JSON keys in the HTTP request body.
- **`RETURN`** is mandatory and terminates the body.
- A deployed query at name `getUser` is callable via `POST /getUser`.
- Append `?` to a parameter name to make it optional (`name?: String`); absent parameters are `NONE`.
- Edge creation (`AddE`) can be used as a statement without assignment when the return value is not needed.

---

## Pattern library

### 1. Node lookup by ID

```hql
QUERY GetUser (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
```

Lookup multiple IDs in one call: `N<User>(id1, id2, id3)`.
Lookup by indexed field: `N<User>({email: email})` — requires `INDEX email` in the schema.

---

### 2. Edge traversal — outbound

`Out<EdgeType>` follows outgoing edges and returns the **destination nodes**.

```hql
QUERY GetFollowing (user_id: ID) =>
    following <- N<User>(user_id)::Out<Follows>
    RETURN following
```

Chain multiple hops inline: `N<User>(user_id)::Out<Follows>::Out<Created>`.
Traverse multiple edge types as a union: `node::Out<EdgeType1, EdgeType2>`.
To get the **edge objects** instead of the destination nodes, use `OutE<Follows>`.

---

### 3. Edge traversal — inbound

`In<EdgeType>` follows incoming edges and returns the **source nodes**.

```hql
QUERY GetFollowers (user_id: ID) =>
    followers <- N<User>(user_id)::In<Follows>
    RETURN followers
```

To get the edge objects, use `InE<Follows>`.
`FromN` and `ToN` navigate from an edge object to its source or destination node.

---

### 4. Vector similarity search — `SearchV`

Searches standalone `V::` vector types by approximate nearest-neighbour.

```hql
QUERY SearchDocs (query_vec: [F64], limit: I64) =>
    docs <- SearchV<Document>(query_vec, limit)
    RETURN docs
```

Auto-embed a string using the configured embedding provider with `Embed()`:

```hql
#[model(text-embedding-3-small)]
QUERY SearchByText (text: String, limit: I64) =>
    docs <- SearchV<Document>(Embed(text), limit)
    RETURN docs
```

Post-filter results after search: `SearchV<Document>(query_vec, limit)::WHERE(_::{created_at}::GTE(cutoff))`.

---

### 5. Node-field vector search — `SearchN`

Searches over a `vector(N)` field declared directly on a node type. The dot-notation `NodeType.fieldName` identifies which embedding field to search.

```hql
// Schema: N::Article { title: String, embedding: vector(1536) }
QUERY FindSimilarArticles (query: [F64], k: I32) =>
    results <- SearchN<Article.embedding>(query, k)
    RETURN results
```

Returns `N::Article` nodes ranked by cosine similarity. Query vector must match the declared dimension.

---

### 6. BM25 full-text search — `SearchBM25`

Keyword search using BM25 ranking over an indexed text field.

```hql
QUERY FullTextSearch (query: String, limit: I64) =>
    results <- SearchBM25<Article>(query, limit)
    RETURN results
```

---

### 7. Hybrid search + rerank with RRF

Combine vector similarity search and reranking with Reciprocal Rank Fusion:

```hql
QUERY HybridSearch (query_vec: [F64]) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankRRF
        ::RANGE(0, 10)
    RETURN results
```

With a custom `k` constant (default 60):

```hql
QUERY HybridSearchCustomK (query_vec: [F64], k_val: F64) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankRRF(k: k_val)
        ::RANGE(0, 10)
    RETURN results
```

For diversity-aware reranking use `RerankMMR` (lambda is required — 0 = max diversity, 1 = max relevance):

```hql
QUERY DiverseSearch (query_vec: [F64]) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankMMR(lambda: 0.7)
        ::RANGE(0, 10)
    RETURN results
```

Optional `distance` metric for MMR: `"cosine"` (default), `"euclidean"`, `"dotproduct"`.

---

### 8. Filtered traversal — `WHERE`, `AND`, `OR`

```hql
QUERY GetActiveAdults () =>
    users <- N<User>::WHERE(
        AND(
            _::{age}::GT(18),
            _::{status}::EQ("active")
        )
    )
    RETURN users
```

Comparison operators: `GT`, `GTE`, `LT`, `LTE`, `EQ`, `NEQ`, `CONTAINS`, `IS_IN`.
`EXISTS` / `!EXISTS` test whether a traversal returns any results:

```hql
QUERY GetPopularUsers () =>
    popular <- N<User>::WHERE(EXISTS(_::In<Follows>))
    RETURN popular
```

`INTERSECT` returns elements that appear in every result of a sub-traversal — useful for "has ALL tags":

```hql
QUERY ArticlesByAllTags (tag_names: [String]) =>
    articles <- N<Tag>::WHERE(_::{name}::IS_IN(tag_names))::INTERSECT(_::In<HasTag>)
    RETURN articles
```

---

### 9. Aggregation — `COUNT`, `GROUP_BY`, `ORDER<Asc|Desc>`

```hql
QUERY GetUserStats (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user::{
        name,
        follower_count: _::In<Follows>::COUNT,
        following_count: _::Out<Follows>::COUNT,
    }
```

Sort and paginate:

```hql
QUERY GetRecentPosts () =>
    posts <- N<Post>::ORDER<Desc>(_::{created_at})::RANGE(0, 20)
    RETURN posts
```

Group into a map from field value to array of elements:

```hql
QUERY GroupUsersByAge () =>
    users <- N<User>
    RETURN users::GROUP_BY(age)
```

`FIRST` returns only the first element of the pipeline.

---

### 10. Shortest path

Three algorithms. All chain `::To(destinationId)` to specify the target.

```hql
// BFS — minimum hops (ShortestPath and ShortestPathBFS are identical)
QUERY GetShortestPath (from_id: ID, to_id: ID) =>
    path <- N<City>(from_id)::ShortestPath<Road>::To(to_id)
    RETURN path

// Dijkstra — minimum total weight
QUERY GetFastestRoute (from_id: ID, to_id: ID) =>
    path <- N<City>(from_id)::ShortestPathDijkstras<Road>(_::{distance_km})::To(to_id)
    RETURN path

// A* — heuristic-guided (weight expression + heuristic field name on the node)
QUERY GetAStarRoute (start: ID, end: ID) =>
    path <- N<City>(start)::ShortestPathAStar<Road>(_::{distance}, "h")::To(end)
    RETURN path
```

Weight expressions support the full math function set (`ADD`, `MUL`, `DIV`, etc.) and can be nested.

---

## MCP tool exposure

Annotate a query with `#[mcp]` to expose it as an MCP (Model Context Protocol) tool, making it callable by LLM agents.

```hql
#[mcp]
QUERY get_user_name (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
```

Use `#[model("name")]` (or `#[model(name)]` without quotes) to set the embedding model for `Embed()` calls within the query:

```hql
#[mcp]
#[model(text-embedding-3-small)]
QUERY SearchDocuments (query: String, limit: I64) =>
    docs <- SearchV<Document>(Embed(query), limit)
    RETURN docs
```

The `#[model(...)]` macro tells the gateway which provider model to call when auto-embedding text. Without it, `Embed()` will fail if no default model is configured. Macros appear before the `QUERY` keyword; multiple macros can be stacked.

---

## Type system

### Scalar types

| Type | Description | Notes |
|------|-------------|-------|
| `I8` | Signed 8-bit integer | −128 to 127 |
| `I16` | Signed 16-bit integer | −32,768 to 32,767 |
| `I32` | Signed 32-bit integer | |
| `I64` | Signed 64-bit integer | |
| `U8` | Unsigned 8-bit integer | 0 to 255 |
| `U16` | Unsigned 16-bit integer | |
| `U32` | Unsigned 32-bit integer | |
| `U64` | Unsigned 64-bit integer | |
| `U128` | Unsigned 128-bit integer | |
| `F32` | 32-bit float | IEEE 754 single-precision |
| `F64` | 64-bit float | IEEE 754 double-precision |
| `String` | UTF-8 text | |
| `Boolean` | Boolean | `true` / `false` |

### Special types

| Type | Description |
|------|-------------|
| `ID` | Node/edge/vector identifier — **UUID-based, not a String** |
| `Date` | UTC timestamp |
| `NOW` | Current UTC timestamp (default value expression only) |

### Complex types

| Type | Description | Example |
|------|-------------|---------|
| `[T]` | Array of type T | `[String]`, `[F64]`, `[ID]` |
| `{fields}` | Inline object / struct | `{name: String, age: U32}` |
| `vector(N)` | Fixed-dimension float embedding field on a node | `vector(1536)` |

`vector(N)` is only valid in `N::` node schema definitions. Using it on `E::` edges is a compile error (E111).

---

## Field remapping

Remapping shapes the response without changing storage.

### Property access

```hql
RETURN user::{name, age}           // select specific fields
RETURN user::{displayName: name}   // rename a field
RETURN user::ID                    // extract identifier
```

### Spread operator `..`

Include all remaining fields not yet named explicitly:

```hql
RETURN user::{
    userID: ID,
    ..
}
```

### Exclude fields `!{...}`

Return everything **except** the listed fields:

```hql
QUERY GetWithoutSecret () =>
    files <- N<File>
    RETURN files::!{text}
```

`!{fields}` affects the **response shape only** — all fields remain in storage.

### Computed fields in remapping

```hql
RETURN user::{
    name,
    follower_count: _::In<Follows>::COUNT,
    post_count: _::Out<Created>::COUNT,
}
```

### Closure remapping `|var|{...}`

When the inner expression needs to reference both an outer variable and the current element, use a closure:

```hql
QUERY GetUserPosts (user_id: ID) =>
    user  <- N<User>(user_id)
    posts <- user::Out<HasPost>
    RETURN user::|usr|{
        posts: posts::{
            postID: ID,
            creatorID: usr::ID,
            creatorName: usr::{name},
            ..
        }
    }
```

`|usr|` binds the outer `user` so it is accessible as `usr` inside the remapping block.

---

## Gotchas

1. **`ID` is UUID, not String.** Never pass a raw string where `ID` is expected — the JSON value must be a UUID string (e.g. `"550e8400-e29b-41d4-a716-446655440000"`). Mixing types silently coerces or errors depending on context.

2. **Vector dimension mismatch causes VECTOR_ERROR.** If you insert a `vector(1536)` field but supply an array with a different length, the operation fails with a dimension mismatch error. Always validate embedding dimensions client-side before calling the gateway.

3. **Soft-delete accumulation in HNSW.** `DROP` on a node marks its vector as inactive in the HNSW index but does **not** compact the index. Stale (soft-deleted) entries accumulate over time and can degrade search quality and performance. Periodic index compaction is required for long-lived deployments with heavy deletion.

4. **`AddN` vs `UpsertN` on duplicate IDs.** `AddN` errors if a node with the same ID already exists. `UpsertN` creates the node if absent or merges the provided fields if it exists. Use `UpsertN` whenever idempotency is needed.

5. **`!{fields}` excludes from response only, not storage.** Excluded fields are still written to and read from the database — they just do not appear in the query response. There is no way to partially project at the storage level.

---

## Operator quick-reference

### Mutation

| Operator | Description |
|----------|-------------|
| `AddN<Type>({fields})` | Create a node; errors on duplicate ID |
| `UpsertN({fields})` | Create or merge a node |
| `node::UPDATE({fields})` | Update specific fields; returns updated node |
| `AddE<Type>::From(a)::To(b)` | Create a directed edge |
| `AddE<Type>({props})::From(a)::To(b)` | Create edge with properties |
| `UpsertE({props})::From(a)::To(b)` | Create or update an edge |
| `AddV<Type>(vec, {fields})` | Create a vector entry |
| `UpsertV(vec, {fields})` | Create or update a vector entry |
| `BatchAddV<Type>(collection)` | Bulk insert vectors from an array variable |
| `DROP expression` | Delete node(s), edge(s), or vector(s) |
| `Embed(text)` | Convert string to vector using configured model |

### Traversal

| Operator | Description |
|----------|-------------|
| `N<Type>` | All nodes of a type |
| `N<Type>(id)` | Node by ID |
| `N<Type>({field: val})` | Node by indexed field value |
| `E<Type>` / `E<Type>(id)` | All edges / edge by ID |
| `V<Type>` / `V<Type>(id)` | All vectors / vector by ID |
| `node::Out<EdgeType>` | Outgoing neighbour nodes |
| `node::In<EdgeType>` | Incoming neighbour nodes |
| `node::OutE<EdgeType>` | Outgoing edge objects |
| `node::InE<EdgeType>` | Incoming edge objects |
| `edge::FromN` | Source node of an edge |
| `edge::ToN` | Destination node of an edge |
| `edge::FromV` / `edge::ToV` | Source / destination vector of an edge |

### Filter / control

| Operator | Description |
|----------|-------------|
| `WHERE(predicate)` | Keep elements matching predicate |
| `AND(p1, p2, ...)` | Logical AND of predicates |
| `OR(p1, p2, ...)` | Logical OR of predicates |
| `EXISTS(traversal)` / `!EXISTS(...)` | Test traversal produces results |
| `INTERSECT(_::subTraversal)` | Elements present in all sub-traversal results |
| `GT`, `GTE`, `LT`, `LTE` | Numeric comparisons |
| `EQ`, `NEQ` | Equality / inequality |
| `CONTAINS(s)` | String contains substring |
| `IS_IN(arr)` | Value is in array |

### Vector / search

| Operator | Description |
|----------|-------------|
| `SearchV<Type>(vec, k)` | ANN search over standalone vector type |
| `SearchN<Type.field>(vec, k)` | ANN search over node-field embeddings |
| `SearchBM25<Type>(text, k)` | BM25 full-text search |
| `RerankRRF` / `RerankRRF(k: n)` | Reciprocal Rank Fusion reranker |
| `RerankMMR(lambda: n)` | Maximal Marginal Relevance reranker (lambda required) |

### Aggregation

| Operator | Description |
|----------|-------------|
| `COUNT` | Count elements in pipeline |
| `RANGE(start, end)` | Paginate — slice `[start, end)` |
| `ORDER<Asc>(expr)` / `ORDER<Desc>(expr)` | Sort ascending / descending |
| `FIRST` | Take first element |
| `GROUP_BY(field)` | Group into map of field value → elements |
| `AGGREGATE_BY(fields...)` | Aggregate by one or more fields |

### Path algorithms

| Operator | Description |
|----------|-------------|
| `ShortestPath<E>::To(id)` | BFS — minimum hops |
| `ShortestPathBFS<E>::To(id)` | BFS — identical to `ShortestPath` |
| `ShortestPathDijkstras<E>(weight)::To(id)` | Minimum total weight |
| `ShortestPathAStar<E>(weight, "hField")::To(id)` | Heuristic-guided A* search |

### Remapping

| Syntax | Description |
|--------|-------------|
| `::{field}` | Access a field |
| `::{alias: expr}` | Rename or compute a field |
| `..` | Spread — include all remaining fields |
| `::!{fields}` | Exclude named fields from response |
| `::|var|{...}` | Closure — bind current element to a named variable |
| `::ID` | Extract the element's identifier |

---

## See also

- Full language reference: [docs/HQL.md](../HQL.md)
- HTTP API and deployment: [docs/HTTP_API.md](../HTTP_API.md)
