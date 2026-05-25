---
description: Pre-flight Rust code review with SparrowDB-specific invariant checks
allowed-tools: Bash, Read, Edit
argument-hint: "[optional: specific files or changes to focus on]"
---

# Rust Code Review

Use the `rust-reviewer` subagent to conduct a comprehensive pre-flight review before merging.

## Pre-flight checks

Run these commands to establish the baseline:

```bash
cargo check --workspace --features lmdb,server
cargo clippy --workspace --features lmdb,server -- -D warnings
cargo fmt --check
```

All three must pass green before review proceeds.

## SparrowDB-specific CRITICAL checks

These are blocking issues and must be fixed before merge:

1. **`std::process::Command` in async code**: Search for `std::process::Command` in async functions. Must use `tokio::process::Command` instead. Reference: `crates/sparrow-cli/src/docker.rs` contains sync-only uses which are acceptable.
   
2. **`write_txn()` outside WorkerPool**: All LMDB write mutations must go through the dedicated writer thread. Never open a `write_txn()` outside of the writer thread path (`WorkerPool::process_write()`). If adding a new mutation endpoint, mark it as a write route.

3. **`use sparrow_core::` imports**: All imports must be `use sparrow_db::...`, never `use sparrow_core::...`. The library name override in `sparrow-core/Cargo.toml` is non-negotiable. Verify with: `grep -r "use sparrow_core" crates/`

## Review triage

Findings are classified by severity:

- **CRITICAL**: Blocks merge. Examples: SparrowDB invariant violations above, undefined behavior, data loss paths
- **HIGH**: Strongly recommend fix. Examples: memory leaks, correctness bugs, performance cliffs
- **MEDIUM**: Nice-to-have improvements. Examples: code clarity, test coverage, documentation

## Approval logic

- **CRITICAL or HIGH findings**: Request changes; do not approve
- **MEDIUM findings only**: Approve with suggestions (comment for record)

## Trigger

Run this command:
- Before merging any Rust change to main
- After opening a pull request in the workspace
- When code review finds unexpected issues
