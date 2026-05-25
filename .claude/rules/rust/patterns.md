# Rust Architectural Patterns

## Newtype Pattern

Wrap primitive IDs and domain values in newtypes to prevent accidental argument swaps and make invalid states unrepresentable.

```rust
// Prefer this — compiler prevents swapping NodeId and EdgeId
struct NodeId(u64);
struct EdgeId(u64);

// Not this — nothing stops fn foo(EdgeId, NodeId) being called as foo(node, edge)
fn foo(node: u64, edge: u64) { ... }
```

Derive or implement common traits (`Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`) on newtypes as appropriate.

## Enum State Machines

Model state as enums so illegal states are unrepresentable at the type level. Use exhaustive `match` — never `_ =>` unless the catchall branch has a comment explaining what it covers and why a wildcard is appropriate.

```rust
enum ConnectionState { Connecting, Connected, Disconnected }

match state {
    ConnectionState::Connecting => { ... }
    ConnectionState::Connected  => { ... }
    ConnectionState::Disconnected => { ... }
    // No `_ =>` — adding a new variant will cause a compile error forcing all match arms to be updated
}
```

## Builder Pattern

For structs with many optional fields, use a dedicated builder type with a `build()` method that validates input and returns `Result<T, Error>`. A builder must never panic in `build()` — it must return an error instead.

```rust
let conn = ConnectionBuilder::new()
    .host("localhost")
    .port(7474)
    .build()?;  // Returns Err if configuration is invalid
```

## Repository / Trait Abstraction

Encapsulate all storage access behind a trait. The `sparrow-core` storage engine is the concrete implementation; callers depend on the trait, not the concrete type. This keeps callers testable by enabling mock implementations.

```rust
trait NodeStore {
    fn insert(&mut self, node: NodeRecord) -> Result<NodeId, StoreError>;
    fn get(&self, id: NodeId) -> Result<Option<NodeRecord>, StoreError>;
}
```

## Service Layer

Inject dependencies through constructors. Async service functions take `&self` and receive all dependencies at construction time rather than as function arguments. Do not use process-global state or lazy statics for dependencies.

```rust
struct QueryService { store: Arc<dyn NodeStore + Send + Sync> }

impl QueryService {
    pub fn new(store: Arc<dyn NodeStore + Send + Sync>) -> Self { Self { store } }
    pub async fn run(&self, query: &Query) -> Result<QueryResult, QueryError> { ... }
}
```

## Sealed Traits

When a trait must not be implemented by external code (e.g., a marker trait used for blanket impls), use the sealed trait pattern: a private `Sealed` supertrait in a private module.

```rust
mod private { pub trait Sealed {} }

pub trait MyPublicTrait: private::Sealed { ... }
```

External crates cannot name `private::Sealed`, so they cannot implement `MyPublicTrait`.

## Error Enum Variants

Use specific, descriptive variant names. Avoid `Unknown`, `Other`, or `Generic` unless the error genuinely cannot be classified — and even then, capture the source error inside.

```rust
// Good
#[derive(thiserror::Error, Debug)]
enum StoreError {
    #[error("node {0} not found")] NodeNotFound(NodeId),
    #[error("write transaction conflict")] WriteTxnConflict,
    #[error("lmdb error: {0}")] Lmdb(#[from] heed::Error),
}

// Avoid
enum StoreError { Unknown(String) }
```

## `From` and `Into` Implementations

Implement `From<SpecificError>` for the crate's error type (or use `#[from]` with `thiserror`) rather than writing `.map_err(|e| CrateError::Variant(e))` at every call site.

```rust
#[derive(thiserror::Error, Debug)]
enum AppError {
    #[error("storage error: {0}")]
    Store(#[from] StoreError),  // enables `?` to convert StoreError -> AppError automatically
}
```
