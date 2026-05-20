# CI Workflow Rewrite

**Date:** 2026-05-20  
**Branch:** fix/hnsw-stability-fixes  
**Status:** Implemented

## Problem

All existing CI workflows were inherited from the `helix-db` era and were broken against the current codebase:

- Test workflows ran `cd helix-db` (crate is now `sparrow-db/`)
- Path triggers watched `helix-macros/**` and `helix-db/**` (neither exists)
- `cli_tests.yml` was fully commented out
- `dashboard_check.yml` referenced a `helix-container --features dev` mode that no longer exists
- `hql_tests.yml` used the deprecated `actions-rs/toolchain@v1` action
- Three separate test workflows for feature flag variants (no matrix)

## Decisions

- **No Windows:** SparrowDB's LMDB backend doesn't compile on Windows; dropped from all matrices
- **Single feature-matrix workflow:** Replaced three separate test files with one matrix over `[lmdb, dev-instance, production]`
- **Skip concurrency tests:** Kept consistent with original approach — loom tests are too slow for CI
- **HQL tests preserved:** `hql-tests/run.sh` still works; only fixed path triggers and replaced deprecated toolchain action
- **CLI tests added:** `sparrow-cli/src/tests/` had no CI coverage at all; added `sparrow-cli-tests.yml`

## Files

### Deleted
| File | Reason |
|------|--------|
| `cli_tests.yml` | Fully commented out |
| `dashboard_check.yml` | `helix-container --features dev` no longer exists |
| `db_tests.yml` | Replaced by `sparrow-db-tests.yml` |
| `dev_instance_tests.yml` | Merged into feature matrix |
| `production_db_tests.yml` | Merged into feature matrix |

### Created
| File | Purpose |
|------|---------|
| `sparrow-db-tests.yml` | `cargo test --release -p sparrow-db` × `[lmdb, dev-instance, production]` × `[ubuntu, macos]` |
| `sparrow-cli-tests.yml` | `cargo test -p sparrow-cli --lib` × `[ubuntu, macos]` |

### Modified
| File | Changes |
|------|---------|
| `clippy_check.yml` | Removed Windows, inlined clippy command from `clippy_check.sh`, added `fail-fast: false` |
| `hql_tests.yml` | Fixed path triggers, replaced `actions-rs/toolchain@v1` → `dtolnay/rust-toolchain@stable`, upgraded `cache@v3` → `v4`, moved `${{ matrix.batch }}` into env var |

### Kept unchanged
`cli.yml`, `cliv2.yml`, `macos-x64-quickfix.yml`, `s3_push.yml` — release/binary workflows, not broken.

## Workflow Details

### `sparrow-db-tests.yml`
- **Triggers:** PRs to `main` touching `sparrow-db/**`, `sparrow-macros/**`, `Cargo.toml`, `Cargo.lock`
- **Matrix:** `os × features` = 2 × 3 = 6 jobs
- **Command:** `cargo test --release -p sparrow-db --features "$FEATURES" --lib -- --skip concurrency_tests`
- **Cache key:** includes feature name to avoid cross-contamination

### `sparrow-cli-tests.yml`
- **Triggers:** PRs to `main` touching `sparrow-cli/**`, `sparrow-db/**`, `sparrow-macros/**`, `Cargo.toml`, `Cargo.lock`
- **Matrix:** 2 OS jobs
- **Command:** `cargo test -p sparrow-cli --lib`

### `clippy_check.yml`
- **Triggers:** All PRs to `main`
- **Matrix:** 2 OS jobs
- **Command:** `cargo clippy --workspace --locked --exclude hql-tests --exclude metrics` with `-D warnings` and standard allows

### `hql_tests.yml`
- **Triggers:** PRs to `main` touching `hql-tests/**`, `sparrow-db/**`, `sparrow-macros/**`, `Cargo.toml`, `Cargo.lock`
- **Matrix:** 10 batch jobs (100 tests split evenly)
- **Command:** `./run.sh batch 10 "$BATCH"` from `./hql-tests/` working directory
