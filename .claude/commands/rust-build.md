---
description: Diagnose and resolve cargo build failures in the SparrowDB workspace
allowed-tools: Bash, Read, Edit
argument-hint: "[optional: describe the build error]"
---

# Rust Build Resolver

Use the `rust-build-resolver` subagent to diagnose and fix cargo compilation errors.

## Workspace diagnostic command

Run this first to identify all build failures:

```bash
cargo check --workspace --features lmdb,server
```

This workspace-aware check respects all feature flags and will catch errors across:
- `sparrow-core` (lib name: `sparrow_db`)
- `sparrow-cli`
- `sparrow-container`
- `sparrow-macros`
- `sparrow-memory`
- `sparrow-metrics`
- `sparrow-benches`
- `sparrow-studio`
- `sdks/rust`
- `tests/hql-tests`

## Approach

1. **Diagnose**: Run the check command above; copy the full error output
2. **Fix one error at a time**: Read the error location, make a minimal change
3. **Re-check immediately**: Re-run the check to confirm the fix worked
4. **Repeat**: Keep fixing until `cargo check` passes green
5. **Lint**: Once green, run `cargo clippy --workspace --features lmdb,server -- -D warnings`
6. **Test**: If there are tests, run `cargo test --workspace --features lmdb,server`

## SparrowDB constraints (never violate these)

- **`sparrow_db` library name**: Never remove `[lib] name = "sparrow_db"` from `crates/sparrow-core/Cargo.toml`. All imports must be `use sparrow_db::...`, never `use sparrow_core::...`
- **`tokio::process::Command` only**: Never use `std::process::Command` in async code; it blocks the Tokio runtime. Sync helpers in `crates/sparrow-cli/src/docker.rs` are OK, but any async refactor must switch to `tokio::process::Command`
- **No `#[allow()]` workarounds**: Fix the actual problem, not the warning
- **No `unsafe` shortcuts**: If a borrow checker error feels unsolvable, it's a design issue; refactor rather than use `unsafe`

## Trigger

Run this command when:
- `cargo build` or `cargo check` fails
- You pull changes that break compilation
- You see borrow checker or type errors after modifying code
