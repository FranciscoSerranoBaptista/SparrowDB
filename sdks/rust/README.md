# sparrow-sdk (Rust)

Rust client SDK for SparrowDB. Provides a query-builder DSL for constructing read/write batch requests and an async `reqwest`-based HTTP client for executing them against a running SparrowDB instance.

## Add to your project

```toml
[dependencies]
sparrow-sdk = { path = "sdks/rust" }   # local monorepo
# or once published:
# sparrow-sdk = "1.0"
```

## Quick start

```rust
use sparrow_sdk::{Client, dsl::prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new("http://localhost:6969", None)?;

    let response = client.run_query(
        read_batch()
            .var_as("users", g().n_with_label("User").limit(10).value_map(["$id", "name"]))
            .returning(["users"])
    ).await?;

    println!("{response:?}");
    Ok(())
}
```

## DSL modules

| Module | Description |
|---|---|
| `sparrow_sdk::dsl` | Traversal builders (`g()`, `read_batch()`, `write_batch()`, `var_as(...)`, node/edge constructors) |
| `sparrow_sdk::dsl::prelude` | Re-exports all common DSL types — use `use sparrow_sdk::dsl::prelude::*` |
| `sparrow_sdk::query_generator` | Bundle generation: `defineQueries`, `registerRead`, `registerWrite`, serialisation helpers |
| `sparrow_sdk::Client` | Async HTTP client. Also available as `sparrow_sdk::SparrowDBClient` (backwards-compatible alias) |

## Build

```bash
cargo build -p sparrow-sdk
```

## Test

```bash
cargo test -p sparrow-sdk
```

## Key files

| File | Description |
|---|---|
| `src/lib.rs` | Crate root: re-exports DSL surface, defines `Client` and `SparrowError` |
| `src/dsl.rs` | Full query-builder DSL implementation |
| `src/query_generator.rs` | Query bundle generation and serialisation |
