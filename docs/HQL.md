# HQL Language Reference

**HQL** (Hypergraph Query Language) is the query and schema definition language for SparrowDB. It is imperative and statement-based: each query is a sequence of variable assignments that are executed top-to-bottom, terminated by a `RETURN` statement. Operations are composed by chaining steps with the `::` operator.

HQL source files use the `.hx` extension. A `.hx` file can contain schema definitions, migration definitions, and query definitions in any order.

---

## Table of Contents

1. [Key Concepts](#1-key-concepts)
2. [Quick Start](#2-quick-start)
3. [Schema Definition](#3-schema-definition)
4. [Query Definitions](#4-query-definitions)
5. [Variables and Assignment](#5-variables-and-assignment)
6. [Node Operations](#6-node-operations)
7. [Edge Operations](#7-edge-operations)
8. [Vector Operations](#8-vector-operations)
9. [Graph Traversal](#9-graph-traversal)
10. [Filtering](#10-filtering)
11. [Aggregation and Sorting](#11-aggregation-and-sorting)
12. [Shortest Path](#12-shortest-path)
13. [Mathematical Functions](#13-mathematical-functions)
14. [Vector Reranking](#14-vector-reranking)
15. [Field Remapping and Object Construction](#15-field-remapping-and-object-construction)
16. [Loops](#16-loops)
17. [Return Values](#17-return-values)
18. [Type Reference](#18-type-reference)
19. [Migrations](#19-migrations)
20. [Comments](#20-comments)
21. [Appendix: Parser Notes](#21-appendix-parser-notes)

---

## 1. Key Concepts

### The `::` operator

The double-colon `::` is the **step separator**. It is used to chain operations on a value — traversal steps, filters, remappings, aggregations, and mutations — into a pipeline. It has nothing to do with Rust's path separator.

```hql
// Each :: advances the pipeline one step
result <- N<User>(id)::Out<Follows>::WHERE(_::{age}::GT(18))::RANGE(0, 10)
```

### Anonymous traversal `_`

Inside filter predicates and remapping expressions, `_` refers to the **current element** being iterated. `_::{}` accesses a property of the current element. `_::Out<Edge>` traverses from the current element.

```hql
// _ means "the current node being examined"
adults <- N<User>::WHERE(_::{age}::GT(18))
```

### Identifiers and type names

- **Variable names** and **field names**: start with a lowercase letter or underscore, followed by alphanumeric or `_`. Example: `user`, `user_id`, `my_field`.
- **Type names** (node, edge, vector type identifiers): start with an uppercase letter. Example: `User`, `Follows`, `Document`.

### Mutability

HQL has no "update in place" concept at the language level. `UPDATE`, `UpsertN`, `UpsertE`, and `UpsertV` are explicit mutation steps that return the mutated entity.

---

## 2. Quick Start

A complete social network example demonstrating schema + queries:

**schema.hx**
```hql
N::User {
    name: String,
    age: U32,
    email: String,
    created_at: Date DEFAULT NOW,
}

N::Post {
    content: String,
    created_at: Date DEFAULT NOW,
}

E::Follows {
    From: User,
    To: User,
    Properties: {
        since: Date DEFAULT NOW,
    }
}

E::Created {
    From: User,
    To: Post,
}
```

**queries.hx**
```hql
// Create a user
QUERY createUser (name: String, age: U32, email: String) =>
    user <- AddN<User>({name: name, age: age, email: email})
    RETURN user

// Follow another user
QUERY createFollow (follower_id: ID, followed_id: ID) =>
    follower <- N<User>(follower_id)
    followed <- N<User>(followed_id)
    AddE<Follows>::From(follower)::To(followed)
    RETURN "success"

// Create a post
QUERY createPost (user_id: ID, content: String) =>
    user <- N<User>(user_id)
    post <- AddN<Post>({content: content})
    AddE<Created>::From(user)::To(post)
    RETURN post

// Get posts from users a given user follows, with remapped fields
QUERY getFollowedUsersPosts (user_id: ID) =>
    following <- N<User>(user_id)::Out<Follows>
    posts <- following::Out<Created>::RANGE(0, 40)
    RETURN posts::{
        post: _::{content},
        creatorID: _::In<Created>::ID,
    }
```

Deploy these files with `sparrow push`, then call them over HTTP:

```
POST /createUser
{"name": "Alice", "age": 30, "email": "alice@example.com"}
```

---

## 3. Schema Definition

Schemas declare the node types, edge types, and vector types that exist in the database. A schema lives in a `.hx` file alongside queries.

### Node types

```hql
N::TypeName {
    fieldName: FieldType,
    ...
}
```

**Example:**
```hql
N::User {
    name: String,
    age: U8,
    email: String,
}
```

Nodes are identified by an auto-generated `ID`. They can have zero or more typed fields.

### Edge types

Edges are directed and connect two named node (or vector) types. Edge properties are optional.

```hql
E::TypeName {
    From: SourceType,
    To: TargetType,
    Properties: {
        fieldName: FieldType,
        ...
    }
}
```

**Example — edge without properties:**
```hql
E::Follows {
    From: User,
    To: User,
}
```

**Example — edge with properties:**
```hql
E::Friends {
    From: User,
    To: User,
    Properties: {
        since: Date,
        strength: F64,
    }
}
```

The `Properties:` block is optional. If an edge has no properties, omit it entirely or leave the block empty.

### Vector types

Vectors hold floating-point embeddings alongside optional metadata fields. They are created with `AddV` and searched with `SearchV`.

```hql
V::TypeName {
    fieldName: FieldType,
    ...
}
```

**Example:**
```hql
V::Document {
    content: String,
    created_at: Date,
}
```

### Indexes

Add `INDEX` before a field declaration to create a lookup index on that field. Indexed fields can be used for O(1) lookup by value (see [N by index](#n-by-index)).

```hql
N::User {
    INDEX email: String,
    name: String,
    age: U32,
}
```

### Unique indexes

`UNIQUE INDEX` enforces uniqueness on a field — an insert that would create a duplicate indexed value will fail.

```hql
N::Account {
    UNIQUE INDEX username: String,
    email: String,
}
```

### Unique edges

A `UNIQUE` modifier on an edge type prevents duplicate edges between the same pair of nodes.

```hql
E::Follows UNIQUE {
    From: User,
    To: User,
}
```

### Default values

Fields can have a default value applied when no value is provided at creation time.

```hql
N::Post {
    content: String,
    created_at: Date DEFAULT NOW,
    views: U32 DEFAULT 0,
    published: Boolean DEFAULT false,
    title: String DEFAULT "Untitled",
}
```

Supported default value expressions:

| Default | Meaning |
|---------|---------|
| `NOW` | Current UTC timestamp (for `Date` fields) |
| `0`, `42`, `-1` | Integer literal |
| `3.14` | Float literal |
| `true` / `false` | Boolean literal |
| `"text"` | String literal |
| `NONE` | Null / absent value |

### Schema versioning

For managed migrations (see [Migrations](#19-migrations)), wrap schema definitions in a version block:

```hql
schema::1 {
    N::User {
        name: String,
        age: U32,
    }
}
```

---

## 4. Query Definitions

Queries are named, callable procedures. They accept typed parameters and return a value.

### Basic syntax

```hql
QUERY queryName (param1: Type, param2: Type) =>
    // body: zero or more assignment statements
    variable <- expression
    RETURN variable
```

### Parameters

Parameters are positional and typed. They are passed as JSON keys in the HTTP request body.

```hql
QUERY GetUser (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
```

```hql
QUERY CreateUser (name: String, age: U8, email: String) =>
    user <- AddN<User>({name: name, age: age, email: email})
    RETURN user
```

### Optional parameters

Append `?` after a parameter name to make it optional. If not provided, the parameter is `NONE`.

```hql
QUERY SearchUsers (name?: String, min_age?: U32) =>
    users <- N<User>
    RETURN users
```

### No parameters

Use empty parentheses:

```hql
QUERY GetAllUsers () =>
    users <- N<User>
    RETURN users
```

### Macros

Macros appear as attributes before the `QUERY` keyword.

#### `#[mcp]`

Exposes the query as an MCP (Model Context Protocol) tool, making it callable by LLM agents.

```hql
#[mcp]
QUERY get_user_name (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
```

#### `#[model(name)]`

Specifies the embedding model to use for `Embed()` calls within this query.

```hql
#[model(text-embedding-3-small)]
QUERY SearchDocuments (query: String, limit: I64) =>
    docs <- SearchV<Document>(Embed(query), limit)
    RETURN docs
```

---

## 5. Variables and Assignment

Variables are assigned using `<-`:

```hql
variableName <- expression
```

Variables are scoped to the query body. The same variable can be reassigned. Variables can reference earlier variables in subsequent expressions.

```hql
QUERY GetFriendsOfFriends (user_id: ID) =>
    user    <- N<User>(user_id)
    friends <- user::Out<Knows>
    fof     <- friends::Out<Knows>
    RETURN fof
```

An assignment result can be discarded (result used for side-effects only, like creating an edge):

```hql
QUERY CreateFollow (from_id: ID, to_id: ID) =>
    from_user <- N<User>(from_id)
    to_user   <- N<User>(to_id)
    AddE<Follows>::From(from_user)::To(to_user)   // no assignment needed
    RETURN "success"
```

---

## 6. Node Operations

### Select all nodes of a type

```hql
nodes <- N<TypeName>
```

Returns every node of the given type.

```hql
QUERY GetAllUsers () =>
    users <- N<User>
    RETURN users
```

### Select a node by ID

```hql
node <- N<TypeName>(id)
```

`id` can be a variable of type `ID` or a string literal.

```hql
QUERY GetUser (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
```

### <a name="n-by-index"></a>Select a node by indexed field value

```hql
node <- N<TypeName>({indexedField: value})
```

Requires that `indexedField` is declared with `INDEX` in the schema.

```hql
QUERY GetUserByEmail (email: String) =>
    user <- N<User>({email: email})
    RETURN user

// Float-indexed lookup
QUERY GetByCount (count: F32) =>
    node <- N<File>({count: count})
    RETURN node
```

### Select multiple nodes by multiple IDs

```hql
nodes <- N<TypeName>(id1, id2, id3)
```

### Create a node — `AddN`

```hql
node <- AddN<TypeName>({field: value, ...})
```

Creates a new node and returns it. Fields not provided will use their `DEFAULT` value if declared, or be absent.

```hql
// With properties
QUERY CreateUser (name: String, age: U8, email: String) =>
    user <- AddN<User>({name: name, age: age, email: email})
    RETURN user

// With literal values
QUERY CreateSystemUser () =>
    user <- AddN<User>({name: "system", age: 0, email: "system@internal"})
    RETURN user

// Empty node (no fields, or all fields have defaults)
QUERY CreateEmptyNode () =>
    node <- AddN<MyType>
    RETURN node
```

### Update a node — `UPDATE`

`UPDATE` is a chained step (last step) that modifies the fields of a node and returns the updated node.

```hql
updated <- node::UPDATE({field: newValue, ...})
```

```hql
QUERY UpdateUserAge (user_id: ID, new_age: U32) =>
    updated <- N<User>(user_id)::UPDATE({age: new_age})
    RETURN updated

QUERY UpdateProfile (user_id: ID, name: String, age: U32) =>
    updated <- N<User>(user_id)::UPDATE({name: name, age: age})
    RETURN updated
```

`Update` (capitalized differently) is also accepted.

### Upsert a node — `UpsertN`

`UpsertN` creates the node if it does not exist, or updates it if it does. It is a chained last step applied to a node or node collection:

```hql
result <- existing::UpsertN({field: value, ...})
```

```hql
// Upsert by finding first (using WHERE), then upserting
QUERY UpsertUser (name: String, age: U32) =>
    existing <- N<User>::WHERE(_::{name}::EQ(name))
    user <- existing::UpsertN({name: name, age: age})
    RETURN user

// Upsert by ID
QUERY UpsertUserById (id: ID, new_age: U32) =>
    existing <- N<User>(id)
    user <- existing::UpsertN({age: new_age})
    RETURN user
```

### Delete — `DROP`

`DROP` deletes a node, edge, vector, or any traversal result. It is a statement (not an assignment).

```hql
DROP expression
```

```hql
// Delete a node by ID
QUERY DeleteUser (user_id: ID) =>
    DROP N<User>(user_id)
    RETURN "Removed"

// Delete all outgoing neighbors
QUERY DeleteFollowing (user_id: ID) =>
    DROP N<User>(user_id)::Out<Follows>
    RETURN "Removed outgoing neighbors"

// Delete outgoing edges only (not the neighbor nodes)
QUERY DeleteFollowEdges (user_id: ID) =>
    DROP N<User>(user_id)::OutE<Follows>
    RETURN "Removed edges"

// Delete incoming neighbors
QUERY DeleteFollowers (user_id: ID) =>
    DROP N<User>(user_id)::In<Follows>
    RETURN "Removed followers"
```

---

## 7. Edge Operations

### Select all edges of a type

```hql
edges <- E<TypeName>
```

### Select an edge by ID

```hql
edge <- E<TypeName>(id)
```

### Select edges by indexed property

```hql
edges <- E<TypeName>({propertyField: value})
```

```hql
QUERY GetFollowEdge (edge_id: ID) =>
    edge <- E<Follows>(edge_id)
    RETURN edge

QUERY GetAllFollows () =>
    follows <- E<Follows>
    RETURN follows
```

### Create an edge — `AddE`

```hql
edge <- AddE<TypeName>::From(fromNode)::To(toNode)
edge <- AddE<TypeName>({prop: value})::From(fromNode)::To(toNode)
```

`From` and `To` accept a node variable, an edge variable, a vector variable, or an `ID` value. The `From::To` and `To::From` orderings are both valid.

```hql
// Simple edge without properties
QUERY CreateFollow (user1_id: ID, user2_id: ID) =>
    edge <- AddE<Follows>::From(user1_id)::To(user2_id)
    RETURN edge

// Edge with properties
QUERY CreateFriendship (user1_id: ID, user2_id: ID) =>
    edge <- AddE<Friends>({since: "2024-01-15", strength: 0.85})::From(user1_id)::To(user2_id)
    RETURN edge

// Edge between nodes resolved by traversal
QUERY FollowByName (follower_id: ID, target_name: String) =>
    target <- N<User>::WHERE(_::{name}::EQ(target_name))
    edge <- AddE<Follows>::From(follower_id)::To(target)
    RETURN edge

// To::From ordering (equivalent to From::To)
QUERY CreateEdgeToFrom (a_id: ID, b_id: ID) =>
    edge <- AddE<Knows>::To(b_id)::From(a_id)
    RETURN edge
```

### Upsert an edge — `UpsertE`

`UpsertE` creates the edge if it does not exist, or updates its properties if it does.

```hql
edge <- existing::UpsertE({prop: value, ...})::From(fromNode)::To(toNode)
```

```hql
QUERY UpsertFriendship (id1: ID, id2: ID, since: String, strength: F32) =>
    person1  <- N<Person>(id1)
    person2  <- N<Person>(id2)
    existing <- E<Friends>
    edge     <- existing::UpsertE({since: since, strength: strength})::From(person1)::To(person2)
    RETURN edge
```

---

## 8. Vector Operations

Vectors store floating-point embeddings with optional metadata. They participate in ANN (approximate nearest-neighbor) search.

### Create a vector — `AddV`

```hql
vec <- AddV<TypeName>(vectorData)
vec <- AddV<TypeName>(vectorData, {field: value, ...})
```

`vectorData` is one of:
- A variable of type `[F64]` (array of floats)
- A literal float array: `[1.0, 0.5, -0.3]`
- `Embed(text)` — auto-embed a string using the configured embedding provider

```hql
// From a float array parameter
QUERY InsertVector (vector: [F64], content: String, created_at: Date) =>
    doc <- AddV<Document>(vector, {content: content, created_at: created_at})
    RETURN doc

// Literal vector (e.g. for testing)
QUERY InsertLiteralVector () =>
    doc <- AddV<Document>([1.0, 0.0, 0.5], {content: "test"})
    RETURN doc

// Auto-embed text (requires embedding provider configured)
QUERY InsertText (content: String) =>
    doc <- AddV<Document>(Embed(content), {content: content})
    RETURN doc

// Vector with no metadata
QUERY InsertBareVector (vector: [F64]) =>
    doc <- AddV<Document>(vector)
    RETURN doc
```

### Batch insert vectors — `BatchAddV`

Insert a collection of vectors from a variable containing an array.

```hql
BatchAddV<TypeName>(vectorsVariable)
```

```hql
QUERY BatchInsert (embeddings: [{ vector: [F64], content: String }]) =>
    BatchAddV<Document>(embeddings)
    RETURN "done"
```

### Select vectors

```hql
vecs <- V<TypeName>
vec  <- V<TypeName>(id)
vecs <- V<TypeName>({indexedField: value})
```

Works identically to node selection.

### Vector similarity search — `SearchV`

Finds the `k` nearest vectors to a query vector.

```hql
results <- SearchV<TypeName>(queryVector, k)
```

`k` can be an integer literal or a variable of integer type.

```hql
// Search with a float array parameter
QUERY SearchDocs (query_vec: [F64], limit: I64) =>
    docs <- SearchV<Document>(query_vec, limit)
    RETURN docs

// Search with auto-embedding
QUERY SearchByText (text: String, limit: I64) =>
    docs <- SearchV<Document>(Embed(text), limit)
    RETURN docs

// Search then filter (post-filter on metadata)
QUERY SearchRecent (query_vec: [F64], limit: I64, cutoff: Date) =>
    docs <- SearchV<Document>(query_vec, limit)::WHERE(_::{created_at}::GTE(cutoff))
    RETURN docs

// Search as part of a traversal chain
QUERY SearchViaGraph (query_vec: [F64], k: I32) =>
    vecs       <- N<SubChapter>::Out<EmbeddingOf>::SearchV<Embedding>(Embed(query_vec), k)
    RETURN vecs
```

### Node-field vector search — `SearchN`

Searches over a `vector(N)` field declared directly on a node type (rather than a separate `V::` type).

```hql
results <- SearchN<NodeType.fieldName>(queryVector, k)
```

```hql
// Schema: N::Person { embedding: vector(1536), name: String }
QUERY SearchPeople (query: [F64], k: I32) =>
    people <- SearchN<Person.embedding>(query, k)
    RETURN people
```

> **Note:** `SearchN` is parsed and compiled, but runtime execution support is still being finalized. Check release notes for current status.

### Full-text search — `SearchBM25`

BM25 keyword search over an indexed text field.

```hql
results <- SearchBM25<TypeName>(queryString, k)
```

```hql
QUERY FullTextSearch (query: String, limit: I64) =>
    results <- SearchBM25<Article>(query, limit)
    RETURN results
```

### Upsert a vector — `UpsertV`

Update the vector and/or metadata of an existing vector entry, or create it if absent.

```hql
result <- existing::UpsertV(newVector, {field: value, ...})
```

```hql
QUERY UpsertDocument (content: String, vector: [F64]) =>
    existing <- V<Document>::WHERE(_::{content}::EQ(content))
    doc      <- existing::UpsertV(vector, {content: content})
    RETURN doc

// Auto-embed on upsert
QUERY UpsertDocumentEmbed (text: String) =>
    existing <- V<Document>::WHERE(_::{content}::EQ(text))
    doc      <- existing::UpsertV(Embed(text), {content: text})
    RETURN doc
```

### `Embed()`

Converts a string to a vector using the configured embedding provider. Usable wherever a vector argument is accepted.

```hql
Embed(stringVariable)
Embed("literal text")
```

---

## 9. Graph Traversal

Traversal steps are chained with `::` onto a node, edge, or vector to navigate the graph.

### `Out<EdgeType>` — outgoing neighbors

Returns all nodes reachable via outgoing edges of the given type.

```hql
neighbors <- node::Out<EdgeType>
```

Omit the type to traverse all outgoing edges:

```hql
all_neighbors <- node::Out
```

Multiple edge types (union):

```hql
neighbors <- node::Out<EdgeType1, EdgeType2>
```

```hql
QUERY GetFollowing (user_id: ID) =>
    following <- N<User>(user_id)::Out<Follows>
    RETURN following

QUERY GetPosts (user_id: ID) =>
    posts <- N<User>(user_id)::Out<Created>
    RETURN posts
```

### `In<EdgeType>` — incoming neighbors

Returns all nodes that have an outgoing edge of the given type pointing to this node.

```hql
incomers <- node::In<EdgeType>
```

```hql
QUERY GetFollowers (user_id: ID) =>
    followers <- N<User>(user_id)::In<Follows>
    RETURN followers
```

### `OutE<EdgeType>` — outgoing edges

Returns the **edge objects** (not neighbor nodes) for outgoing edges of the given type.

```hql
edges <- node::OutE<EdgeType>
```

```hql
QUERY GetFollowEdges (user_id: ID) =>
    edges <- N<User>(user_id)::OutE<Follows>
    RETURN edges
```

### `InE<EdgeType>` — incoming edges

Returns the **edge objects** for incoming edges of the given type.

```hql
edges <- node::InE<EdgeType>
```

```hql
QUERY GetFollowerEdges (user_id: ID) =>
    edges <- N<User>(user_id)::InE<Follows>
    RETURN edges
```

### `FromN` — source node of an edge

Returns the **node** at the `From` end of an edge.

```hql
source <- edge::FromN
```

```hql
QUERY GetCreatorFromEdge (creation_id: ID) =>
    creator <- E<Created>(creation_id)::FromN
    RETURN creator
```

### `ToN` — destination node of an edge

Returns the **node** at the `To` end of an edge.

```hql
target <- edge::ToN
```

```hql
QUERY GetFollowedUser (follow_id: ID) =>
    followed <- E<Follows>(follow_id)::ToN
    RETURN followed
```

### `FromV` — source vector of an edge

Returns the **vector** at the `From` end of an edge (when the `From` type is a `V::` type).

```hql
source_vec <- edge::FromV
```

### `ToV` — destination vector of an edge

Returns the **vector** at the `To` end of an edge (when the `To` type is a `V::` type).

```hql
dest_vec <- edge::ToV
```

```hql
QUERY GetDocumentVector (creation_id: ID) =>
    doc_vec <- E<Creates>(creation_id)::ToV
    RETURN doc_vec
```

### Chaining traversals

Multiple traversal steps compose naturally:

```hql
// Two hops: user -> follows -> their posts
QUERY GetFollowedUsersPosts (user_id: ID) =>
    followed <- N<User>(user_id)::Out<Follows>
    posts    <- followed::Out<Created>
    RETURN posts

// Inline chaining (equivalent)
QUERY GetFollowedUsersPosts (user_id: ID) =>
    posts <- N<User>(user_id)::Out<Follows>::Out<Created>
    RETURN posts
```

---

## 10. Filtering

### `WHERE` — conditional filter

`WHERE` is a traversal step that filters to only elements matching a predicate.

```hql
filtered <- source::WHERE(predicate)
```

The predicate is typically an anonymous traversal (`_::...`) applying a comparison operator.

#### Comparison operators

All comparison operators are used as chained last steps on a property access:

```hql
_::{fieldName}::OPERATOR(value)
```

| Operator | Meaning | Example |
|----------|---------|---------|
| `GT(n)` | Greater than | `_::{age}::GT(18)` |
| `GTE(n)` | Greater than or equal | `_::{age}::GTE(18)` |
| `LT(n)` | Less than | `_::{age}::LT(65)` |
| `LTE(n)` | Less than or equal | `_::{age}::LTE(65)` |
| `EQ(v)` | Equal | `_::{name}::EQ("Alice")` |
| `NEQ(v)` | Not equal | `_::{status}::NEQ("banned")` |
| `CONTAINS(s)` | String contains substring | `_::{email}::CONTAINS("@gmail")` |
| `IS_IN(arr)` | Value is in array | `_::{tag}::IS_IN(tag_list)` |

```hql
// Numeric comparisons
QUERY GetAdults () =>
    adults <- N<User>::WHERE(_::{age}::GT(18))
    RETURN adults

QUERY GetUnder30 () =>
    users <- N<User>::WHERE(_::{age}::LT(30))
    RETURN users

// String equality
QUERY GetActiveUsers (status: String) =>
    active <- N<User>::WHERE(_::{status}::EQ(status))
    RETURN active

// String contains
QUERY GetGmailUsers () =>
    users <- N<Users>::WHERE(_::{email}::CONTAINS("@gmail.com"))
    RETURN users

// IS_IN — field value is one of the given array elements
QUERY GetNodesByField (values: [String]) =>
    nodes <- N<MyNode>::WHERE(_::{field}::IS_IN(values))
    RETURN nodes

// IS_IN with IDs
QUERY GetNodesByIds (node_ids: [ID]) =>
    nodes <- N<MyNode>::WHERE(_::{id}::IS_IN(node_ids))
    RETURN nodes

// Date comparison
QUERY GetRecentPosts (cutoff: Date) =>
    posts <- N<Post>::WHERE(_::{created_at}::GTE(cutoff))
    RETURN posts
```

#### Traversal comparison — filter via a related value

You can access a property through a traversal inside `WHERE`:

```hql
// Filter posts whose creation edge has a timestamp after `date`
QUERY GetNewPosts (birthday: Date, date: Date) =>
    posts <- N<User>({birthday: birthday})::Out<Created>::WHERE(_::InE<Created>::{created_at}::GTE(date))
    RETURN posts
```

#### `WHERE` on traversal results

`WHERE` can appear at any point in a chain:

```hql
// Filter after traversal
result <- N<User>(user_id)::Out<Follows>::WHERE(_::{age}::GT(25))

// Filter before traversal
result <- N<User>::WHERE(_::{age}::GT(18))::Out<Created>
```

### `EXISTS` — relationship existence check

Tests whether a traversal returns any results. Returns a boolean.

```hql
EXISTS(traversal)
!EXISTS(traversal)   // negated
```

```hql
// Users who have at least one follower
QUERY GetPopularUsers () =>
    popular <- N<User>::WHERE(EXISTS(_::In<Follows>))
    RETURN popular

// Users who have no friends
QUERY GetLonelyUsers () =>
    lonely <- N<User>::WHERE(!EXISTS(_::Out<Knows>))
    RETURN lonely

// Both at once
QUERY GetHasFriendsSegment () =>
    has_friends    <- N<User>::WHERE(EXISTS(_::Out<Knows>))
    has_no_friends <- N<User>::WHERE(!EXISTS(_::Out<Knows>))
    RETURN has_friends, has_no_friends
```

### `AND` / `OR` — boolean combinators

Combine multiple predicates. Both accept a variadic list.

```hql
AND(predicate1, predicate2, ...)
OR(predicate1, predicate2, ...)
```

They can be negated with `!`:

```hql
!AND(...)
!OR(...)
```

```hql
// Users aged over 18 AND named Alice or Bob
QUERY GetFilteredUsers () =>
    users <- N<User>::WHERE(
        AND(
            _::{age}::GT(18),
            OR(_::{name}::EQ("Alice"), _::{name}::EQ("Bob"))
        )
    )
    RETURN users

// AND with date conditions
QUERY GetNewPostsAnd (birthday: Date, date: Date) =>
    posts <- N<User>({birthday: birthday})::Out<Created>::WHERE(
        AND(
            _::{created_at}::GTE(date),
            _::InE<Created>::{created_at}::GTE(date)
        )
    )
    RETURN posts
```

### `INTERSECT` — set intersection

Returns only elements that appear in **every** result of the given sub-traversal. Used to find items that match all of a set of criteria.

```hql
results <- source::INTERSECT(_::subTraversal)
```

```hql
// Articles that have ALL of the given tags
QUERY ArticlesByAllTags (tag_names: [String]) =>
    articles <- N<Tag>::WHERE(_::{name}::IS_IN(tag_names))::INTERSECT(_::In<HasTag>)
    RETURN articles

// Further filter after intersection
QUERY ArticlesByAllTagsAndTitle (tag_names: [String], name: String) =>
    articles <- N<Tag>
        ::WHERE(_::{name}::IS_IN(tag_names))
        ::INTERSECT(_::In<HasTag>)
        ::WHERE(_::{title}::EQ(name))
    RETURN articles
```

---

## 11. Aggregation and Sorting

### `COUNT` — count elements

Returns the count of elements in the current pipeline as a number.

```hql
n <- source::COUNT
```

```hql
QUERY CountFollowers (user_id: ID) =>
    n <- N<User>(user_id)::In<Follows>::COUNT
    RETURN n

// COUNT inside a remapping expression
QUERY GetUserStats (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user::{
        name,
        follower_count: _::In<Follows>::COUNT,
        following_count: _::Out<Follows>::COUNT,
    }
```

### `RANGE` — pagination

Slices a result set by index range `[start, end)`.

```hql
page <- source::RANGE(start, end)
```

Both bounds can be integer literals or variables.

```hql
QUERY GetPage (page: I64) =>
    posts <- N<Post>::RANGE(0, 20)
    RETURN posts

// Dynamic range from parameters
QUERY GetPageDynamic (offset: I64, limit: I64) =>
    posts <- N<Post>::RANGE(offset, limit)
    RETURN posts
```

### `ORDER` — sort

Sorts elements by a field in ascending or descending order.

```hql
sorted <- source::ORDER<Asc>(_::{field})
sorted <- source::ORDER<Desc>(_::{field})
```

```hql
QUERY GetUsersByAge () =>
    users_asc  <- N<User>::ORDER<Asc>(_::{age})
    users_desc <- N<User>::ORDER<Desc>(_::{age})
    RETURN users_asc, users_desc

// By date, newest first
QUERY GetRecentPosts () =>
    posts <- N<Post>::ORDER<Desc>(_::{created_at})
    RETURN posts
```

### `FIRST` — take first element

Returns only the first element of the pipeline.

```hql
first_item <- source::FIRST
```

```hql
QUERY GetFirstUser () =>
    user <- N<User>::FIRST
    RETURN user
```

### `GROUP_BY` — group by field

Groups elements by the value of one or more fields. Returns a map from field value to array of matching elements.

```hql
grouped <- source::GROUP_BY(fieldName)
grouped <- source::GROUP_BY(field1, field2)
```

```hql
QUERY GroupUsersByAge () =>
    users <- N<User>
    RETURN users::GROUP_BY(age)
```

### `AGGREGATE_BY` — aggregate by field

Aggregates elements by one or more fields.

```hql
aggregated <- source::AGGREGATE_BY(field1, field2)
```

```hql
QUERY AggregateUsersByAge () =>
    users <- N<User>
    RETURN users::AGGREGATE_BY(age)
```

---

## 12. Shortest Path

SparrowDB provides three shortest-path algorithms. All take a starting node, an edge type to traverse, and a destination ID.

### `ShortestPath` / `ShortestPathBFS` — minimum hops

Finds the path with the fewest edges. `ShortestPath` and `ShortestPathBFS` are identical.

```hql
path <- N<NodeType>(startId)::ShortestPath<EdgeType>::To(endId)
path <- N<NodeType>(startId)::ShortestPathBFS<EdgeType>::To(endId)
```

```hql
QUERY GetShortestPath (from_id: ID, to_id: ID) =>
    path <- N<City>(from_id)::ShortestPath<Road>::To(to_id)
    RETURN path
```

### `ShortestPathDijkstras` — minimum total weight

Finds the path with the minimum sum of a specified edge property (the weight).

```hql
path <- N<NodeType>(startId)::ShortestPathDijkstras<EdgeType>(_::{weightField})::To(endId)
```

The weight expression can be any math expression over `_::{edgeProperty}`.

```hql
QUERY GetFastestRoute (from_id: ID, to_id: ID) =>
    path <- N<City>(from_id)::ShortestPathDijkstras<Road>(_::{distance_km})::To(to_id)
    RETURN path

// Custom weight formula
QUERY GetCheapestRoute (from_id: ID, to_id: ID) =>
    path <- N<City>(from_id)
        ::ShortestPathDijkstras<Road>(MUL(_::{distance_km}, _::{toll_factor}))
        ::To(to_id)
    RETURN path
```

### `ShortestPathAStar` — heuristic-guided search

A* search. Takes a weight expression and a heuristic field name (string). The heuristic field must be a numeric field on the **node** type.

```hql
path <- N<NodeType>(startId)::ShortestPathAStar<EdgeType>(_::{weightField}, "heuristicField")::To(endId)
```

```hql
// Schema: N::City { name: String, h: F64 }
//         E::Road { From: City, To: City, Properties: { distance: F64, traffic_factor: F64 } }

QUERY GetAStarRoute (start: ID, end: ID) =>
    path <- N<City>(start)::ShortestPathAStar<Road>(_::{distance}, "h")::To(end)
    RETURN path

// Complex weight formula
QUERY GetAStarRouteCustom (start: ID, end: ID) =>
    path <- N<City>(start)
        ::ShortestPathAStar<Road>(
            MUL(_::{distance}, ADD(1, DIV(_::{traffic_factor}, 10))),
            "h"
        )
        ::To(end)
    RETURN path
```

---

## 13. Mathematical Functions

Mathematical functions use prefix notation and accept numeric expressions as arguments. They can be nested freely.

### Arithmetic (binary)

| Function | Operation | Example |
|----------|-----------|---------|
| `ADD(a, b)` | a + b | `ADD(2, 3)` → 5 |
| `SUB(a, b)` | a - b | `SUB(10, 3)` → 7 |
| `MUL(a, b)` | a × b | `MUL(4, 5)` → 20 |
| `DIV(a, b)` | a ÷ b | `DIV(10, 2)` → 5 |
| `POW(a, b)` | a ^ b | `POW(2, 8)` → 256 |
| `MOD(a, b)` | a mod b | `MOD(10, 3)` → 1 |

### Unary math

| Function | Operation |
|----------|-----------|
| `ABS(x)` | Absolute value |
| `SQRT(x)` | Square root |
| `CEIL(x)` | Ceiling |
| `FLOOR(x)` | Floor |
| `ROUND(x)` | Round to nearest integer |
| `LN(x)` | Natural logarithm |
| `LOG10(x)` | Base-10 logarithm |
| `LOG(x)` | Logarithm (base e) |
| `EXP(x)` | e^x |

### Trigonometry

| Function | Description |
|----------|-------------|
| `SIN(x)` | Sine (radians) |
| `COS(x)` | Cosine (radians) |
| `TAN(x)` | Tangent (radians) |
| `ASIN(x)` | Arcsine |
| `ACOS(x)` | Arccosine |
| `ATAN(x)` | Arctangent |
| `ATAN2(y, x)` | Two-argument arctangent |

### Aggregate functions

These aggregate a collection into a single value.

| Function | Description |
|----------|-------------|
| `MIN(x)` | Minimum value |
| `MAX(x)` | Maximum value |
| `SUM(x)` | Sum of values |
| `AVG(x)` | Average of values |
| `COUNT(x)` | Count of values |

### Constants

| Expression | Value |
|------------|-------|
| `PI()` | π ≈ 3.14159… |
| `E()` | e ≈ 2.71828… |

### Usage in queries

Math functions can be used in remapping expressions, computed fields, weight expressions, and as standalone values.

```hql
// Computed field in remapping
QUERY GetUserStats (user_id: ID) =>
    user <- N<Container>(user_id)
    RETURN user::{
        name,
        total_relations: ADD(_::Out<Contains>::COUNT, _::In<Contains>::COUNT),
        ratio:           DIV(_::Out<Contains>::COUNT, _::In<Contains>::COUNT),
    }

// Nested math
QUERY GetComplexStat (container_id: ID) =>
    container <- N<Container>(container_id)
    RETURN container::{
        deep_calc: MUL(
            ADD(_::Out<Contains>::COUNT, _::In<Contains>::COUNT),
            DIV(_::Out<Contains>::COUNT, 2)
        )
    }

// Math in Dijkstra weight
QUERY GetWeightedPath (from_id: ID, to_id: ID) =>
    path <- N<City>(from_id)
        ::ShortestPathDijkstras<Road>(
            MUL(_::{distance}, ADD(1, DIV(_::{traffic_factor}, 10)))
        )
        ::To(to_id)
    RETURN path
```

---

## 14. Vector Reranking

Rerankers post-process vector search results to improve relevance. They are chained after `SearchV`.

### `RerankRRF` — Reciprocal Rank Fusion

Reranks by combining rank positions. Optional `k` parameter controls the ranking constant (default 60).

```hql
results <- SearchV<Type>(vec, n)::RerankRRF
results <- SearchV<Type>(vec, n)::RerankRRF(k: value)
```

```hql
QUERY SearchRRF (query_vec: [F64]) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankRRF
        ::RANGE(0, 10)
    RETURN results

QUERY SearchRRFCustomK (query_vec: [F64], k_val: F64) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankRRF(k: k_val)
        ::RANGE(0, 10)
    RETURN results
```

### `RerankMMR` — Maximal Marginal Relevance

Reranks to balance relevance and diversity. Required `lambda` controls the relevance-diversity tradeoff (0 = max diversity, 1 = max relevance). Optional `distance` metric.

```hql
results <- SearchV<Type>(vec, n)::RerankMMR(lambda: value)
results <- SearchV<Type>(vec, n)::RerankMMR(lambda: value, distance: "metric")
```

Supported distance metrics: `"cosine"` (default), `"euclidean"`, `"dotproduct"`.

```hql
QUERY SearchMMR (query_vec: [F64]) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankMMR(lambda: 0.7)
        ::RANGE(0, 10)
    RETURN results

QUERY SearchMMREuclidean (query_vec: [F64]) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankMMR(lambda: 0.5, distance: "euclidean")
        ::RANGE(0, 10)
    RETURN results

QUERY SearchMMRVariable (query_vec: [F64], lambda_val: F64) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankMMR(lambda: lambda_val)
        ::RANGE(0, 10)
    RETURN results
```

### Chaining rerankers

Multiple rerankers can be chained; they are applied in order.

```hql
QUERY SearchChained (query_vec: [F64]) =>
    results <- SearchV<Document>(query_vec, 100)
        ::RerankRRF(k: 60)
        ::RerankMMR(lambda: 0.7)
        ::RANGE(0, 10)
    RETURN results
```

---

## 15. Field Remapping and Object Construction

HQL has rich syntax for shaping the output of a query — selecting specific fields, renaming them, computing derived values, and building nested objects.

### Property access — `::{ field }`

Access one or more fields from a node, edge, or vector.

```hql
value  <- node::{fieldName}
values <- node::{field1, field2, field3}
```

```hql
QUERY GetName (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user::{name}

QUERY GetNameAge (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user::{name, age}
```

### Field remapping — `::{ alias: expression }`

Rename fields or compute new values in the output.

```hql
remapped <- node::{
    alias: _::{fieldName},
    computedAlias: expression,
    literalAlias: "constant",
}
```

`_` inside an object step refers to the current element.

```hql
QUERY GetDisplayInfo () =>
    users <- N<User>::RANGE(0, 10)
    RETURN users::{displayName: name, userAge: age}

// Accessing ID explicitly
QUERY GetWithId () =>
    users <- N<User>::RANGE(0, 5)
    RETURN users::{
        userID: ID,
        displayName: name,
        age: age,
    }

// Computed field using math
QUERY GetStats (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user::{
        name,
        follower_count: _::In<Follows>::COUNT,
        post_count: _::Out<Created>::COUNT,
    }
```

### Shorthand field syntax

When the alias and field name are the same, you can write just the field name:

```hql
// These are equivalent:
RETURN user::{name: name, age: age}
RETURN user::{name, age}
```

### `ID` step — extract identifier

`::ID` extracts the identifier of a node/edge/vector.

```hql
id <- node::ID
```

```hql
QUERY GetIds () =>
    ids <- N<User>::ID
    RETURN ids
```

### Spread operator `..`

Includes all remaining fields not explicitly named.

```hql
remapped <- node::{
    userID: ID,
    ..
}
```

```hql
// Include all fields plus a renamed ID
QUERY GetAllFieldsPlusId (user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user::{
        userID: ID,
        ..
    }
```

### Exclude fields — `::!{ field }`

Returns all fields **except** the listed ones.

```hql
result <- node::!{fieldToExclude, anotherField}
```

```hql
QUERY GetWithoutSecret () =>
    files <- N<File>
    RETURN files::!{text}
```

### Closure remapping — `|alias|{...}`

Iterates over a collection with a named variable (alias) for the current element. Useful when the inner expression needs to reference both the outer context and the current element.

```hql
result <- collection::|item|{
    fieldAlias: item::{fieldName},
    computed: EXPRESSION_USING(item),
}
```

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

### Nested object construction

Build arbitrary nested objects and arrays in `RETURN`:

```hql
RETURN {
    key1: value1,
    key2: value2,
    nested: {
        innerKey: innerValue
    },
    list: [item1, item2, item3]
}
```

```hql
QUERY GetAppData (user_id: ID) =>
    app      <- N<App>(user_id)
    branches <- app::Out<HasBranch>
    RETURN {
        app: {
            name: app::{name},
            description: app::{description},
            branches: [branches],
        }
    }
```

---

## 16. Loops

`FOR...IN` iterates over a collection variable, executing a body block for each element.

### Basic loop

```hql
FOR item IN collection {
    // body
}
```

```hql
QUERY CreateItems (names: [String]) =>
    FOR item IN names {
        created <- AddN<Item>({name: item})
    }
    RETURN "Done"
```

### Destructuring loop

Destructure each element into named fields:

```hql
FOR {field1, field2, field3} IN collection {
    // field1, field2, field3 are in scope
}
```

```hql
QUERY CreateDocuments (docs: [{ title: String, content: String }]) =>
    FOR {title, content} IN docs {
        node <- AddN<Document>({title: title, content: content})
    }
    RETURN "Done"
```

### Object access loop

Iterate using dot notation to access a sub-field as the loop variable:

```hql
FOR record.field IN collection {
    // record is bound
}
```

### Nested loops

```hql
FOR {id, subchapters} IN chapters {
    chapter <- AddN<Chapter>({chapter_index: id})
    FOR {title, content, chunks} IN subchapters {
        subchapter <- AddN<SubChapter>({title: title, content: content})
        AddE<Contains>::From(chapter)::To(subchapter)
        FOR {chunk, vector} IN chunks {
            vec <- AddV<Embedding>(vector)
            AddE<EmbeddingOf>({chunk: chunk})::From(subchapter)::To(vec)
        }
    }
}
```

### Complex example — RAG ingestion

```hql
QUERY IngestChapters (
    chapters: [{ id: I64, subchapters: [{ title: String, content: String, chunks: [{chunk: String, vector: [F64]}] }] }]
) =>
    FOR {id, subchapters} IN chapters {
        chapter_node <- AddN<Chapter>({chapter_index: id})
        FOR {title, content, chunks} IN subchapters {
            subchapter_node <- AddN<SubChapter>({title: title, content: content})
            AddE<Contains>::From(chapter_node)::To(subchapter_node)
            FOR {chunk, vector} IN chunks {
                vec <- AddV<Embedding>(vector)
                AddE<EmbeddingOf>({chunk: chunk})::From(subchapter_node)::To(vec)
            }
        }
    }
    RETURN "Success"
```

---

## 17. Return Values

Every query must end with `RETURN`.

### Single variable

```hql
RETURN user
```

### Multiple variables (tuple)

```hql
RETURN user, friends, count
```

### String literal

```hql
RETURN "success"
RETURN "Removed"
```

### Array

```hql
RETURN [user1, user2, user3]
```

### Object

```hql
RETURN {
    user: user,
    count: count,
    status: "ok",
}
```

### Nested object with arrays

```hql
RETURN {
    app: {
        branches: [
            {
                name: dev_branch::{name},
                frontend: {
                    page_folders: [
                        {
                            name: main_folder::{name},
                            pages: [index_page, not_found_page]
                        }
                    ]
                }
            }
        ],
        name: app::{name},
        description: app::{description},
    }
}
```

### Remapping in RETURN

Remapping steps can be applied directly on the `RETURN` expression:

```hql
// Return with field selection
RETURN user::{name, age}

// Return collection with remapped fields
RETURN posts::{
    post: _::{content},
    creatorID: _::In<Created>::ID,
}

// Return with aggregation
RETURN users::GROUP_BY(age)
```

---

## 18. Type Reference

### Scalar types

| Type | Description | Range / Notes |
|------|-------------|---------------|
| `String` | UTF-8 text | Arbitrary length |
| `Boolean` | Boolean | `true` or `false` |
| `F32` | 32-bit float | IEEE 754 single-precision |
| `F64` | 64-bit float | IEEE 754 double-precision |
| `I8` | Signed 8-bit integer | −128 to 127 |
| `I16` | Signed 16-bit integer | −32,768 to 32,767 |
| `I32` | Signed 32-bit integer | −2^31 to 2^31−1 |
| `I64` | Signed 64-bit integer | −2^63 to 2^63−1 |
| `U8` | Unsigned 8-bit integer | 0 to 255 |
| `U16` | Unsigned 16-bit integer | 0 to 65,535 |
| `U32` | Unsigned 32-bit integer | 0 to 2^32−1 |
| `U64` | Unsigned 64-bit integer | 0 to 2^64−1 |
| `U128` | Unsigned 128-bit integer | 0 to 2^128−1 |

### Special types

| Type | Description |
|------|-------------|
| `ID` | Node/edge/vector identifier (UUID-based) |
| `Date` | UTC timestamp |
| `NOW` | Current UTC timestamp (default value only) |

### Complex types

| Type | Description | Example |
|------|-------------|---------|
| `[T]` | Array of type T | `[String]`, `[F64]`, `[ID]` |
| `{fields}` | Inline object / struct | `{name: String, age: U32}` |
| `vector(N)` | Fixed-dimension float vector | `vector(1536)` |

### Literals

| Literal kind | Example |
|--------------|---------|
| Integer | `42`, `0`, `-1` |
| Float | `3.14`, `0.0`, `-1.5` |
| String | `"hello"`, `"2024-01-15"` |
| Boolean | `true`, `false` |
| Float array | `[1.0, 0.5, -0.3]` |
| None | `NONE` |

### Using types in query parameters

```hql
QUERY Example (
    name: String,
    age: U32,
    score: F64,
    active: Boolean,
    user_id: ID,
    tags: [String],
    birth_date: Date,
    optional_field?: String,
) =>
    RETURN "ok"
```

---

## 19. Migrations

Migrations transform data when the schema changes. They map old type definitions to new ones, with field-level transformations.

### Syntax

```hql
MIGRATION schema::N => schema::M {
    N::OldType => N::NewType {
        oldField: newValue,
        renamedField: oldFieldName AS NewType,
    }
    E::OldEdge => E::NewEdge { Properties: { prop: value } }
    V::OldVec => V::NewVec { field: value }
    N::DroppedType => _::      // map to anonymous (drop type)
}
```

**Example:**

```hql
MIGRATION schema::1 => schema::2 {
    N::User => N::Person {
        name: name,
        birthYear: age AS I32,
    }
    E::Follows => E::Follows { Properties: { weight: 1 } }
}
```

Migration fields support:
- Direct field copy: `fieldName: fieldName`
- Renamed field: `newName: oldName`
- Type cast: `newName: oldName AS TargetType`
- Literal default: `fieldName: "default_value"`
- Timestamp: `fieldName: NOW`

---

## 20. Comments

HQL supports single-line comments with `//`. Comments can appear anywhere whitespace is allowed.

```hql
// This is a full-line comment

N::User {
    name: String, // inline comment
    age: U32,
}

QUERY GetUser (user_id: ID) =>
    // Fetch the user by ID
    user <- N<User>(user_id)
    RETURN user // return the result
```

---

## 21. Appendix: Parser Notes

> This section is for contributors to SparrowDB, not application developers.

### Source files

| File | Purpose |
|------|---------|
| `crates/sparrow-core/src/grammar.pest` | PEG grammar (authoritative syntax definition) |
| `crates/sparrow-core/src/sparrowc/parser/` | Parser modules (pest → AST) |
| `crates/sparrow-core/src/sparrowc/analyzer/` | Semantic analysis and type inference |
| `crates/sparrow-core/src/sparrowc/generator/` | Code generation (AST → runtime bytecode) |

### Grammar overview

The grammar is written in [pest](https://pest.rs/) PEG format. The top-level rule is `source`:

```pest
source = { SOI ~ (schema_def | migration_def | query_def)* ~ EOI }
```

### Key grammar rules

| Rule | What it parses |
|------|----------------|
| `schema_def` | `N::`, `E::`, `V::` declarations (optionally in `schema::N {}`) |
| `query_def` | `QUERY name(params) => body RETURN expr` |
| `get_stmt` | `identifier <- evaluates_to_anything` |
| `traversal` | Pipeline starting from a start node/edge/vector |
| `step` | Single `::` step (graph, filter, remap, aggregate, etc.) |
| `last_step` | Terminal step (comparison, UPDATE, UpsertN, UpsertE, UpsertV, FIRST) |
| `start_node` | `N<Type>(id)`, `N<Type>`, `N<Type>{field:val}` |
| `start_edge` | `E<Type>(id)`, `E<Type>` |
| `start_vector` | `V<Type>(id)`, `V<Type>` |
| `search_vector` | `SearchV<Type>(vec, k)` |
| `search_node_vector` | `SearchN<Type.field>(vec, k)` |
| `bm25_search` | `SearchBM25<Type>(query, k)` |
| `AddN` | `AddN<Type>({fields})` |
| `AddE` | `AddE<Type>({props})::From(a)::To(b)` |
| `AddV` | `AddV<Type>(vec, {fields})` |
| `for_loop` | `FOR arg IN ident { body }` |
| `object_step` | `{field: expr, ..}` (remapping) |
| `closure_step` | `\|ident\| object_step` |
| `exclude_field` | `!{field1, field2}` |
| `where_step` | `WHERE(predicate)` |
| `order_by` | `ORDER<Asc\|Desc>(expr)` |
| `bool_operations` | `GT`, `GTE`, `LT`, `LTE`, `EQ`, `NEQ`, `CONTAINS`, `IS_IN` |
| `rerank_rrf` | `RerankRRF(k: n)?` |
| `rerank_mmr` | `RerankMMR(lambda: n, distance?: s)` |
| `shortest_path` | `ShortestPath<E>::To(id)` |
| `shortest_path_dijkstras` | `ShortestPathDijkstras<E>(expr)::To(id)` |
| `shortest_path_astar` | `ShortestPathAStar<E>(expr, "field")::To(id)` |
| `math_function_call` | `FUNC(args...)` |
| `return_stmt` | `RETURN expr (,expr)*` |

### Parser module layout

| Module | Responsibility |
|--------|----------------|
| `query_parse_methods.rs` | Top-level query parsing (`query_def`, `query_params`, `query_body`) |
| `creation_step_parse_methods.rs` | `AddN`, `AddE`, `AddV`, `BatchAddV` |
| `traversal_parse_methods.rs` | Traversal pipelines, start nodes, graph steps, shortest path, `SearchN` |
| `graph_step_parse_methods.rs` | `Out`, `In`, `OutE`, `InE`, `FromN`, `ToN`, `FromV`, `ToV` |
| `expression_parse_methods.rs` | `math_function_call`, `bool_operations`, `AND`, `OR`, `EXISTS` |
| `return_value_parse_methods.rs` | `return_stmt`, array/object construction |
| `object_parse_methods.rs` | `object_step`, `closure_step`, `exclude_field`, `spread_object` |
| `schema_parse_methods.rs` | Schema and migration definitions |
| `migration_parse_methods.rs` | Migration body parsing |

### Identifier naming conventions in the grammar

- **Type identifiers** (`identifier_upper`): must start with an uppercase ASCII letter — `User`, `Follows`, `Document`. Used for node, edge, vector type names.
- **Value identifiers** (`identifier`): start with a lowercase letter — `user`, `user_id`, `my_field`. Used for variables, field names, parameter names.

### Anonymous traversal `_`

Inside filter predicates and remapping steps, `_` is the **anonymous traversal** rule. It refers to "the current element" without binding it to a named variable. The anonymous traversal can chain any number of steps.

The grammar defines two kinds of traversal:
- `id_traversal`: `identifier :: step* :: last_step?` — traversal starting from a named variable
- `anonymous_traversal`: `_ :: step* :: last_step?` — traversal starting from the current element

### Step vs. last step

Steps in the grammar are split into two categories:

- **`step`** (`::` prefix): intermediate steps that can appear anywhere in a chain — graph navigation, `WHERE`, `ORDER`, remapping, `COUNT`, `RANGE`, `AddE`, rerankers.
- **`last_step`** (`::` prefix): terminal steps that must appear at the end of a chain — comparison operators (`GT`, `EQ`, etc.), `UPDATE`, `UpsertN`, `UpsertE`, `UpsertV`, `FIRST`.

This distinction is enforced by the grammar to prevent ambiguous chains.

### Feature flags

HQL compilation requires the `compiler` feature:

```
compiler = pest + pest_derive + ariadne
```

The `ariadne` crate provides human-readable error reporting. It **must** remain in the feature graph whenever `compiler` is enabled — its removal silently breaks error diagnostics.

Full gateway + storage requires:

```
lmdb = server = build + compiler + vectors + heed3
```

See `crates/sparrow-core/CLAUDE.md` for the complete feature flag chain.
