# sparrow-macros

Procedural macro crate providing attribute macros and derive macros for SparrowDB.

---

## Critical constraint: proc-macro crate

`[lib] proc-macro = true` — this is **not** a normal library.

- **Cannot be listed under `[dependencies]`** in any crate. It must appear under
  `[dependencies]` with `proc-macro` semantics (Cargo handles this automatically when the
  crate declares `proc-macro = true`, but do not add it as a regular lib dep).
- **Cannot import from `sparrow_db` or any `sparrow-*` library crate.** Proc-macro crates
  link into the compiler, not the final binary — circular linkage will produce cryptic linker
  errors at build time.
- **Macro expansion errors surface in the consumer crate** (`sparrow-core`), not here.
  If `cargo check` on this crate passes but `sparrow-core` fails with a strange token error,
  the bug is in the generated code, not in the consumer.

---

## Macros provided

| Macro | Kind | Purpose |
|-------|------|---------|
| `#[handler(is_write?)]` | attribute | Registers an HTTP handler via `inventory::submit!` into the router |
| `#[get_handler]` | attribute | Like `#[handler]` but hardcodes `is_write = false` |
| `#[mcp_handler]` | attribute | Registers an MCP handler via `inventory::submit!` |
| `#[tool_calls]` | attribute | Expands a trait into per-method MCP handler functions + `#[mcp_handler]` wrappers |
| `#[tool_call(name, txn_type)]` | attribute | Generates an MCP wrapper function with a read or write transaction |
| `#[migration(Type, from -> to)]` | attribute | Registers a schema migration transition via `inventory::submit!` |
| `#[sparrow_node]` | attribute | Injects an `id: String` field into a struct |
| `#[derive(Traversable)]` | derive | Adds an `id()` accessor; requires an `id` field or emits `compile_error!` |

---

## Testing

Macros are tested through `sparrow-core`. To verify a macro change:

```bash
cargo test --package sparrow-core --features lmdb,server
```

For macro expansion debugging (requires `cargo install cargo-expand`):

```bash
cargo expand --package sparrow-core --features lmdb,server <module>
```

`trybuild` is available as a dev-dependency for compile-error snapshot tests if needed.

---

## Agent invocation guide

| Situation | Agent |
|-----------|-------|
| Code review before merging | `.agents/rust-reviewer` |
| Build failure — link errors, proc-macro loading | `.agents/rust-build-resolver` |
| Macro silently generates wrong or incomplete code | `.agents/silent-failure-hunter` |

---

## Skills reference

- **Debugging macro expansion** → `docs/skills/debugging.md` §A (compile error section)

---

## Code graph (MCP tools)

| Tool | When to use |
|------|-------------|
| `get_architecture_overview_tool` | Understand how all macros are structured and what code they generate |
| `get_impact_radius_tool` | Pass a macro name to see every call site across the codebase |
| `semantic_search_nodes_tool` | Find a specific macro implementation by description |
| `get_minimal_context_tool` | Understand a single macro proc in isolation |
