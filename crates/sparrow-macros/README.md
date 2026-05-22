# sparrow-macros

Procedural macros for SparrowDB. Provides attribute and derive macros used by `sparrow-core` and user crates to register HTTP handlers, MCP tools, graph node/edge types, and version-transition functions with the `inventory`-based static registration system.

## Build

```bash
cargo build -p sparrow-macros
```

## Test

```bash
cargo test -p sparrow-macros
```

## Key macros

| Macro | Usage |
|---|---|
| `#[handler]` | Register an async function as an HTTP route handler. Use `#[handler(is_write)]` for write routes. |
| `#[handler(is_write)]` | Same as `#[handler]` but marks the route as a write operation |
| Derive macros | Node/edge type registration for the graph engine |

## Feature flags

| Feature | Description |
|---|---|
| `debug-output` | Emit extra debug info from macro expansion (mirrors `sparrow-core/debug-output`) |

## Usage example

```rust
use sparrow_macros::handler;

#[handler]
async fn get_user(req: Request, db: Arc<Engine>) -> Response {
    // ...
}

#[handler(is_write)]
async fn create_user(req: Request, db: Arc<Engine>) -> Response {
    // ...
}
```
