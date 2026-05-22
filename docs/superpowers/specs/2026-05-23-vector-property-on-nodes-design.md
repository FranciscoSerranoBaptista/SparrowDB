# Design: `vector(N)` Property Type on `N::` Nodes

**Date:** 2026-05-23  
**Status:** Approved  
**Goal:** Add first-class `vector(N)` property syntax to SparrowDB's HQL grammar so embedding fields on graph nodes can be declared inline and auto-indexed into the HNSW engine — eliminating the need for manual `CreateVectorIndexNodes` runtime calls.

---

## Context

SparrowDB already has `V::` for standalone vector documents. These share a single global HNSW index keyed by label (the `V::` type name). However, there is no way to declare an embedding field directly on an `N::` node type. Users who want semantic/vector search over graph nodes today must either use standalone `V::` documents (disconnected from the graph) or issue `CreateVectorIndexNodes` SDK steps that have no server-side handler and therefore silently do nothing.

---

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Grammar form | `vector(N)` | Concise, explicit dimension, no type param needed |
| Scalar type | Always F32 (stored as F64 in HNSW) | F32 is universal for embeddings; matches V:: behaviour |
| Auto-index on insert | Yes — `AddN` triggers `vectors.insert` | Eliminates manual index management |
| HNSW label convention | `"TypeName.fieldname"` | Slots into existing label machinery, no VectorCore changes |
| Query syntax | `SearchN<Type.field>(query, k)` | Clean mirror of `SearchV`, explicit about node-vs-standalone |
| Valid on | `N::` nodes only | YAGNI; E:: vector support can be added later |
| Property bag | Raw array stored in node properties too | Enables retrieval without index traversal |

---

## Grammar Changes (`grammar.pest`)

```pest
// New type — use the existing `integer` rule; must come BEFORE `identifier`
// to prevent the keyword `vector` being matched as an identifier (ordered choice)
vector_type = { "vector" ~ "(" ~ integer ~ ")" }
param_type  = { named_type | date_type | ID_TYPE | array | object | vector_type | identifier }

// New traversal entry point
search_node_vector = {
    "SearchN" ~ "<" ~ type_dot_field ~ ">" ~
    "(" ~ vector_data ~ "," ~ (integer | identifier) ~ ")"
}
type_dot_field = { identifier_upper ~ "." ~ identifier }

// Wire into existing traversal and evaluates_to_anything rules:
//   traversal          = { (start_node | start_edge | search_vector | search_node_vector | start_vector) ~ step* ~ last_step? }
//   evaluates_to_anything = { ... | search_vector | search_node_vector | bm25_search | ... }
```

**Note:** `AddN` grammar is unchanged — `embedding: [0.1, 0.2, ...]` already parses as a valid `create_field` via `array_literal`. The vector routing happens at the engine layer based on the schema's `FieldType::Vector(N)` declaration.

**Note:** `Embed(...)` in `AddN` is out of scope for this feature — `embed_method` is not in `evaluates_to_anything`. Only literal float arrays and variable references work in `AddN` vector fields today.

---

## Type System Changes (`sparrowc/parser/types.rs`)

Add to `FieldType` enum:
```rust
Vector(usize),   // vector(N) — N is the declared dimension
```

- `Display`: renders as `vector({N})`
- `parse_field_type`: match `Rule::vector_type`, parse the integer child, return `FieldType::Vector(dim)`
- `PartialEq<Value>` for `Vector(N)`: valid if `Value::Array` of floats with length == N

---

## Analyzer Changes (`sparrowc/analyzer/`)

1. **Type check:** `vector(N)` fields rejected on `E::` edge types with a diagnostic:  
   `"vector property '{field}' is not allowed on edge type '{type}'; use N:: nodes"`
2. **Dimension check hint:** When `AddN` is issued with a `vector(N)` field, the analyser can emit a warning if the literal array length doesn't match N (best-effort at compile time; runtime also validates).
3. **Introspection:** `IntrospectionData` serialises vector fields on nodes as `{ "type": "vector", "dim": N }` rather than `Array(F32)`.

---

## Insert Path Changes

### `sparrow_engine/traversal_core/ops/source/add_n.rs`

After properties are stored in LMDB:

1. Look up the node's schema from `Ctx` to find any `FieldType::Vector(N)` fields.
2. For each such field:
   - Extract the value from the property map (must be an array of floats, length N).
   - Convert f32 values to `&[f64]`.
   - Call `storage.vectors.insert(txn, label, floats, properties, arena)` where `label = "TypeName.fieldname"`.
3. If extraction fails (wrong length, wrong type): return a runtime error.
4. The raw array remains in the node's property bag as-is.

---

## New SearchN Traversal

### `sparrow_engine/traversal_core/ops/vectors/search_n.rs`

New `SearchNAdapter` struct:
- Takes `type_name: &str`, `field_name: &str`, `query: &[f64]`, `k: usize`
- Constructs `label = format!("{}.{}", type_name, field_name)`
- Calls `storage.vectors.search(txn, query, k, label, filter, true, arena)`
- Returns `HVector` results (same shape as `SearchV`)
- Wired into the traversal dispatcher alongside `SearchV`

---

## Code Generator Changes (`sparrowc/generator/`)

| Output | `vector(N)` renders as |
|--------|------------------------|
| Rust SDK (`schemas.rs`) | `Vec<f32>` |
| TypeScript (`tsdisplay.rs`) | `Array<number> /** vector(N) */` |

---

## What Does NOT Change

- `VectorCore` / HNSW singleton — no structural change
- `HVector` struct
- `SearchV` / `AddV` / `V::` schema semantics
- LMDB schema / existing databases
- The `vectors` / `vector_data` / `hnsw_out_nodes` LMDB databases

---

## Example HQL (before → after)

**Before (today):** No grammar support; must use SDK runtime calls that have no server handler.

**After:**
```hql
N::Person {
    name: String,
    embedding: vector(1536),
}

// Insert a person with embedding
AddN<Person>(name: "Alice", embedding: Embed("Alice is a software engineer"))

// Semantic search
SearchN<Person.embedding>(Embed("software engineers in SF"), 10)
```

---

## Out of Scope (explicitly)

- `E::` edge vector fields
- Per-field isolated HNSW indexes (global singleton stays)
- `vector<F64>(N)` parameterised precision
- Migration tooling for adding `vector(N)` to existing node types (no schema migration needed — new nodes get indexed, old nodes without the field are unaffected)
