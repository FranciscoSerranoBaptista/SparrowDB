# Rust Coding Style

## Formatter

Always use `rustfmt`. Never commit code that fails `cargo fmt --check`. If a `rustfmt.toml` is present at the workspace root, it governs formatting options — do not override it with inline attributes.

## Linting

Clippy is run with `-D warnings` — all warnings are treated as errors. Do not add `#[allow()]` attributes without a comment explaining why the suppression is justified. Prefer fixing the underlying issue over silencing it.

## Line Width

100 characters per line (the rustfmt default for this project).

## Naming Conventions

| Item | Convention | Example |
|------|-----------|---------|
| Functions and variables | `snake_case` | `fn insert_node()`, `let node_id` |
| Types and traits | `PascalCase` | `struct NodeRecord`, `trait Storage` |
| Constants and statics | `SCREAMING_SNAKE_CASE` | `const MAX_NODES: usize` |
| Lifetimes | short lowercase | `'a`, `'db`, `'txn` |
| Modules | `snake_case`, grouped by domain | `mod query`, `mod storage` |

Organize modules by domain (e.g., `query`, `storage`, `index`), not by type (not `structs`, `traits`, `errors`).

## Immutability First

Default to `let`. Use `let mut` only when mutation is actually required. Prefer returning new values over mutating arguments passed by reference.

## Ownership Patterns

- Prefer `&str` over `String` in function parameters unless ownership transfer is needed.
- Prefer `&[T]` over `Vec<T>` in function parameters.
- Use `Cow<'_, str>` for functions that may or may not need to allocate depending on input.
- Use `self` (take ownership) for builder-pattern methods.
- Use `&self` (borrow) for query/read methods.

## Error Handling

- **Library crates** (`sparrow-core`, `sparrow-macros`): define typed error enums with `thiserror`.
- **Application crates** (`sparrow-cli`, `sparrow-container`): use `anyhow` for contextual error wrapping.
- Never call `.unwrap()` or `.expect()` in production (non-test) code without a comment explaining why the panic is provably unreachable. Reserve for tests and truly unreachable branches.
- **Exception — startup validation**: `.expect()` is acceptable in process-startup code (top of `main()`, config initialisation before any request is served) when a missing value should hard-fail the process immediately. This is intentional fast-fail, not a logic error. Always include a descriptive message: `.expect("SPARROW_API_KEY env var must be set")`. Do not use this exception inside request handlers or library code.
- Prefer `?` propagation over explicit `match Err(e) => return Err(...)` boilerplate.

## Visibility

Default to private. Use `pub(crate)` for items shared within the workspace. Only expose `pub` for genuine public API surface. Avoid accidentally widening visibility — review every `pub` at the module boundary.
