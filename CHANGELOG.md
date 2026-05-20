# Changelog

All notable changes to SparrowDB are documented here.

---

## [Unreleased]

### New Features

**HNSW / Vector Search**
- Enable PREFILTER mode during HNSW traversal for more accurate filtered vector search

**Diagnostics**
- `GET /hnsw-health` — BFS reachability check across the HNSW graph; reports unreachable node count and entry point validity

**Memory (sparrow-memory crate)**
- New `sparrow-memory` crate scaffolded: episodic memory store with opaque ID fields, `TryFrom<&str>` for `Priority`, and `PartialEq` on stored types
- Index name constants and core type definitions

### Bug Fixes

**Vector / HNSW**
- Return `ZeroMagnitudeVector` error instead of dividing by zero on zero-magnitude input vectors
- Propagate non-`EntryPointNotFound` errors in `insert` instead of swallowing them

### Internal

**Tests**
- Reduced oversized LMDB map sizes in test helpers: `hnsw_tests` 512 MB → 64 MB, `bm25_tests` 4 GB → 128 MB; tests now run on any machine without requiring excessive free disk space
- Updated three tests that asserted the old buggy zero-magnitude cosine behavior; `test_hvector_distance_max` now uses anti-parallel vectors, and a new `test_hvector_distance_zero_magnitude_returns_error` documents the correct contract
- Fixed two prune unit tests that used zero-magnitude hub vectors, which became invalid once the zero-magnitude guard was in place

**CI**
- Rewrote all GitHub Actions workflows for the `sparrow-*` crate structure
- Replaced three separate feature-flag test files with a single `sparrow-db-tests.yml` matrix (`lmdb` / `dev-instance` / `production` × `ubuntu` / `macos`)
- Added `sparrow-cli-tests.yml` — CLI unit tests previously had no CI coverage
- Dropped Windows from all matrices; replaced deprecated `actions-rs/toolchain@v1` with `dtolnay/rust-toolchain@stable`
- Fixed `hql_tests.yml` path triggers (`helix-*` → `sparrow-*`)

---

## [3.0.0] — 2026-05-20

### Breaking Changes

This release completes the rename from **HelixDB** to **SparrowDB**.

- All `HELIX_*` environment variables renamed to `SPARROW_*` (e.g. `HELIX_DATA_DIR` → `SPARROW_DATA_DIR`)
- `helix.toml` configuration file renamed to `sparrow.toml`
- `.helix/` project directory renamed to `.sparrow/`
- All `helix-*` crates renamed to `sparrow-*`
- All `Helix*` public types renamed to `Sparrow*`
- CLI binary renamed from `helix` to `sparrow`

### New Features

**Runtime HQL Interpreter**
- Added `/__hql_runtime_eval` HTTP endpoint that evaluates HQL queries at runtime
- Full schema-context-aware parsing and analysis of HQL at request time
- Environment-gated route registration via `SPARROW_RUNTIME_EVAL` flag
- Exposes `hql_schema_raw` in `Config` and `StorageConfig` for runtime use

**RocksDB Backend**
- Optional RocksDB storage backend alongside the existing LMDB backend
- Feature-flagged via `lmdb` / `rocks` Cargo features on `sparrow-db`
- Dual storage: `StorageMethods` trait, `RTxn`/`WTxn` type aliases, `storage_core` modules
- RocksDB secondary index merge operator for correct multi-value key handling
- RocksDB BM25 implementation (`rocks_bm25`) alongside existing `lmdb_bm25`
- RocksDB vector core with full HNSW implementation

**Vector Operations**
- `insert_with_id`: insert a vector with a caller-supplied ID
- `rebuild_vector_index`: rebuild the HNSW index in place, purging soft-deleted entries
- `purge_soft_deleted`: remove all soft-deleted vectors without a full rebuild

**Vector Delete API**
- `POST /vector-soft-delete` — marks a vector deleted without removing index links
- `POST /vector-hard-delete` — permanently removes a vector and its HNSW graph links

**Diagnostics Endpoint**
- `GET /diagnostics` — returns node count, edge count, vector count, and entry point health

**Data Management CLI**
- `sparrow data snapshot` — hot-copy the live database to a target directory (LMDB hot-copy; RocksDB checkpoint fallback)
- `sparrow data clone` — full directory copy of an existing database snapshot
- `sparrow data restore [--force]` — restore from a snapshot; `--force` overwrites an existing database

**`sparrow run` Command**
- Execute the SparrowDB server binary directly on bare metal (no container required)

**TypeScript DSL**
- Added TypeScript query IR types and SDK usage examples from the `python-typescript-dsl` design

### Bug Fixes

**Vector / HNSW**
- Preserve per-vector properties (embedding, metadata) through `rebuild_vector_index`
- Guard entry point drift on soft delete — reassign or clear the entry point when the EP vector is soft-deleted
- Replace soft delete with hard delete in `drop_vector` — eliminates a DROP leak where the HNSW graph retained dangling links
- Enforce `m_max_0` degree limit on back-links in `set_neighbours` — was silently exceeding degree cap on layer 0
- Remove `HashSet` allocation in the prune path; add `debug_assert` for malformed neighbor keys

**BM25**
- Guard `0/0` NaN in `calculate_bm25_score` when an empty document is the first insertion

**RocksDB**
- Enable secondary index merge operator — fixes silent key overwrite under concurrent writes

**Embedding**
- Pre-compute embedding in async context — eliminates the last `block_on` deadlock in `search_vector_text`
- Remove the blocking `fetch_embedding` call — all embedding paths now go through the async pipeline

**MCP**
- Cap `cached_results` at 10 000 entries to prevent OOM on large result sets
- Materialize query results on the first `next()` call — eliminates O(N²) re-execution during pagination

**Value Arithmetic**
- Promote signed + unsigned arithmetic to `i128` — eliminates silent overflow when mixing `u64`/`u128` with `i64`

**CLI**
- Fix `SPARROW_DATA_DIR` path doubling (container was appending `/user` to an already-suffixed path)
- Use `tokio::process::Command` instead of `std::process::Command` in async contexts
- Guard `restore --force` against data loss when the backup is inside the destination directory
- Fix `snapshot` to avoid creating the output directory before DB validation

---

## Prior to 3.0.0

SparrowDB v3.0.0 is forked from **HelixDB** v2. For HelixDB history see the upstream repository.
