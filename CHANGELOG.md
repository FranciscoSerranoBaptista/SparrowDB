# Changelog

All notable changes to SparrowDB are documented here.

---

## [Unreleased]

### New Features

**Schema Migrations**
- On-disk migration log — `_migrations_log` LMDB database records the state of every applied transition
  - `MigrationRecord` stores status (`InProgress` / `Complete`), a `(label, from, to)` checksum, and a Unix timestamp
  - `read_record` / `write_record` helpers for serialised log access
- `run_schema_migrations` runs automatically on database startup, after the endianness migration:
  - Groups `Transition` objects by item label and validates chain contiguity (returns an error on gaps)
  - Marks each transition `InProgress` before writing, then `Complete` after all nodes and edges are updated
  - Batch-writes 1024 items per write transaction; the `node.version == from_version` idempotency guard makes restarts after a crash safe
  - Updates `StorageMetadata` to `WithSchemaVersion` with the highest `to_version` seen across all transitions
- `StorageMetadata::WithSchemaVersion` — new metadata variant (storage version tag `2`) that persists the HQL schema version alongside vector endianness
- `MigrationFn` type alias (`fn(HashMap<String, Value>) -> HashMap<String, Value>`) standardises the property transform signature
- `POST /migrate_status` *(dev-instance)* — returns per-transition log state (`NotRun`, `InProgress`, `Complete`) for all inventory-registered transitions
- `POST /migrate_list` *(dev-instance)* — returns all transitions compiled into the binary via `inventory::iter`
- `sparrow migrate status` — queries the running instance's migration log and pretty-prints JSON
- `sparrow migrate apply` — restarts the instance so any pending migrations execute on the next startup
- `sparrow migrate list` — lists all compiled transitions in the binary
- `sparrow upgrade` — the former `sparrow migrate` v1→v2 project wizard is renamed to `upgrade`; `sparrow migrate` is now the schema-migration subcommand dispatcher

**Auth / TokenStore**
- `TokenStore` — LMDB-backed named token registry stored in a dedicated `{data_parent}/auth/` LMDB environment
  - Three roles: `Admin` (full access + token management), `ReadWrite` (read + write queries), `ReadOnly` (read-only queries)
  - `create(name, role)` — generates a `sparrow_<32 hex chars>` token, stores only the SHA-256 hash; returns the raw token once
  - `revoke(id)` — deletes a token by its 8-char short ID; returns `false` when not found
  - `verify(raw_key)` — constant-time full-store scan using `subtle::ConstantTimeEq`; returns `TokenRecord` on success
  - `list()` — returns all token records (never raw keys)
  - `is_auth_required()` — returns `false` when the store is empty; server runs unauthenticated in dev mode with no tokens
- `seed_legacy` — on startup, seeds `SPARROW_API_KEY` as an Admin token; idempotent and backward-compatible with single-key deployments
- `SparrowGateway` and `AppState` now carry `Arc<TokenStore>` (gated on the `lmdb` feature); auth path derived from `opts.path` automatically
- Auth enforced on all three gateway entry points (`post_handler`, `introspect_schema_handler`, `v1_query_axum_handler`); write routes additionally require `can_write()` role
- Removed `api-key` feature flag — replaced by always-compiled `TokenStore` auth that self-disables when the store is empty
- **Token management REST API** (`#[cfg(feature = "lmdb")]`):
  - `GET /tokens` — list all tokens; Admin role required
  - `POST /tokens` — create a named token with a role; returns the raw token once; Admin role required
  - `DELETE /tokens/{id}` — revoke a token by short ID; Admin role required
  - Bootstrap: when no tokens exist, `POST /tokens` is callable without credentials to create the first Admin token

**Sparrow Studio (embedded web UI)**
- New `sparrow-studio` crate — pre-built React/TypeScript assets served via `rust-embed` at `GET /studio` and `GET /studio/*`
- `studio` feature flag on `sparrow-core` merges the studio router into the main gateway; enabled by default in `sparrow-container`
- **HQL Editor** — CodeMirror 6 editor with SparrowDB syntax highlighting, query execution, and per-instance history
- **Schema Browser** — live introspection of all node and edge types with field names and types via `/introspect`
- **Graph Visualiser** — Cytoscape.js cose-bilkent layout; click any node or edge to explore its properties and connections
- **Diagnostics view** — node count, edge count, and vector stats (total / active / soft-deleted / HNSW edges) with configurable auto-refresh
- **Vectors view** — HNSW health (`/hnsw-health`) and bidirectional edge integrity (`/hnsw-integrity`) checks
- **Connection settings panel** — configure host and port; live connectivity test shows green/red status indicator
- `packages/studio` — Vite + React + TypeScript monorepo package; built assets checked in under `sparrow-studio/dist/`
- pnpm workspace added to the repo root for unified JS dependency management

**Vector Fields on Nodes (`vector(N)`)**
- `vector(N)` is now a first-class property type on `N::` node schemas — declare embedding fields inline without a separate `V::` type or manual index calls
  - Grammar: `vector_type`, `type_dot_field`, and `search_node_vector` rules added to the PEG grammar
  - Parser: `FieldType::Vector(usize)` variant; `SearchNodeVector` AST node with `StartNode::SearchNodeVector` and `ExpressionType::SearchNodeVector` variants
  - Analyzer: `GeneratedType::VectorF32(usize)` — renders `Vec<f32>` in generated Rust structs and `Array<number> /* vector(N) */` in TypeScript
  - E111 compile error if `vector(N)` is used on an `E::` edge type
- **`add_n_with_vectors`** — new engine method that stores a node in LMDB and auto-inserts each `vector(N)` field into the HNSW index in one operation, using the node's UUID as the HNSW entry ID (so no separate ID mapping is needed)
- **`SearchN<Type.field>(query, k)`** — new traversal entry point that performs HNSW nearest-neighbour search over a node's vector field and returns hydrated `N::` nodes ranked by cosine similarity; soft-deleted nodes are silently skipped; an empty index returns zero results rather than an error
- Code generation: `AddN` emits `add_n_with_vectors` when the schema has `vector(N)` fields; `SearchN` emits `search_n` calls
- Docs: `vector(N)` field type and `SearchN` fully documented in `docs/HQL.md`

**HNSW / Vector Search**
- Enable PREFILTER mode during HNSW traversal for more accurate filtered vector search

**Diagnostics**
- `GET /hnsw-health` — BFS reachability check across the HNSW graph; reports unreachable node count and entry point validity; now covers all labels (not just `"default"`)
- `GET /hnsw-integrity` — scans every HNSW edge and verifies bidirectional symmetry; reports `asymmetric_edges` count and overall `symmetric` flag

**Memory (sparrow-memory crate)**
- New `sparrow-memory` crate scaffolded: episodic memory store with opaque ID fields, `TryFrom<&str>` for `Priority`, and `PartialEq` on stored types
- Index name constants and core type definitions

**Bulk Import / Export**
- `sparrow import <FILE> [OPTIONS]` — bulk-load records from a JSON, CSV, or Parquet file into a running SparrowDB instance
  - Format auto-detected from extension (`.json`, `.csv`, `.parquet` / `.pq`); override with `--format json|csv|parquet`
  - `--query <NAME>` — call the same HQL query for every record
  - `--query-column <COL>` — per-record query routing: reads the query name from a named field on each record (the column is stripped before posting); `--query` serves as a fallback for records that omit the column — enables importing mixed node+edge files in a single pass
  - `--workers <N>` — concurrent HTTP workers (default 8); backed by `buffer_unordered` so network latency is hidden behind concurrency
  - `--dry-run` — parse the file and print a routing preview without sending any requests
  - `--on-error continue|abort` — skip failures and finish (default) or stop on first error
  - `--token` / `SPARROW_TOKEN` env var — sets the `x-api-key` header for auth-protected instances
  - Full guide: `docs/import.md`
- `sparrow export <FILE> --query <NAME> [OPTIONS]` — export records from a running SparrowDB instance to a JSON, CSV, or Parquet file
  - POSTs `--params <JSON>` (default `{}`) to `POST /<query>` and extracts the response record array
  - `--key <KEY>` — which top-level key in the response object contains the records; auto-detected when the response has exactly one key, required otherwise
  - `--pretty` — pretty-print JSON output (`.json` format only)
  - `--token` / `SPARROW_TOKEN` env var — sets the `x-api-key` header
  - Full guide: `docs/import.md#export`

### Performance

**v1/query write batching**
- `POST /v1/query` write requests now execute all mutation steps (AddN, AddEdge, UpdateProperties, DropNodes) inside a **single LMDB write transaction** committed once at the end of the request, instead of one transaction-per-step
  - Reduces fsync cost from O(N) to O(1) per request regardless of how many write operations it contains
  - For a client sending 100 AddN steps in one request this eliminates 99 of 100 fsyncs
  - Enables `LIMIT` on write throughput to become the wire RTT + one fsync rather than N × fsync
- Read-only steps (Traverse, LookupByUuid) within the same request reuse the write transaction as a read view via `Deref` coercion, allowing them to observe uncommitted writes from earlier steps in the same request
- The batch is **atomic**: if any step fails, all uncommitted writes are rolled back; previously a failing step left earlier AddN writes permanently committed

### Bug Fixes

**Schema Migrations**
- `TransitionFn.func` now uses `MigrationFn` (`HashMap<String, Value>` → `HashMap<String, Value>`) instead of `ImmutablePropertiesMap` — aligns the stored type with the public API and removes a hidden conversion
- `upgrade_to_node_latest` and `upgrade_to_edge_latest` now bump `node.version` / `edge.version` to `item_info.latest` after applying transitions (was left at the old version number)
- `storage_migration_tests.rs` and `storage_concurrent_tests.rs` were orphan files never included in the module tree; declared with `#[cfg(test)] mod` in `mod.rs` so their tests now actually run
- `#[sparrow_migration]` macro path corrected from `graph_core::ops::version_info` to `storage_core::version_info`

**Auth / TokenStore**
- Constant-time verification uses `subtle::ConstantTimeEq` over a full scan — no early exit after a match prevents timing-based token enumeration
- All write paths serialized with `std::sync::Mutex` — prevents concurrent LMDB write-txn deadlocks from async Axum handlers
- `seed_legacy` is idempotent — unconditional `put` replaces the read-then-write TOCTOU race
- `revoke()` uses the `db.delete` return value instead of hardcoding `true` — correctly reports concurrent deletion
- `require_admin` gated on `lmdb` feature — was missing the gate, causing compile errors on non-lmdb builds
- `seed_legacy` logs `warn!` on LMDB write failures — previously swallowed errors silently, leaving the server unauthenticated with no operator signal
- Auth path derivation handles bare-filename `opts.path` — `Path::parent()` returns `Some("")` for bare names; now treated as absent and falls back to a unique `/tmp` path
- `SparrowError::InvalidApiKey` now maps to HTTP 401 (was 403); new `SparrowError::Forbidden` maps to 403
- `DELETE /tokens/:id` route corrected to `DELETE /tokens/{id}` — Axum 0.8 dropped colon-style path parameters

**Sparrow Studio**
- Parse `/introspect` JSON response correctly in the Studio API client (was treating the raw string as the schema)
- Show connection status indicator in the header when the configured instance is unreachable
- `DiagnosticsResponse` type corrected to match the actual `/diagnostics` response shape (`nodes` / `edges` / `vectors.{total,active,soft_deleted,...}` instead of the old `node_count` / `edge_count` / `db_size_bytes` / `uptime_secs`); Diagnostics view now displays all stats correctly
- Vite dev-proxy now injects the `x-api-key` header from `SPARROW_API_KEY` env var — Studio is fully usable against a token-protected instance without disabling auth
- Default `baseUrl` in the connection store changed from `http://localhost:6969` to `""` (empty = same-origin Vite proxy); direct browser-to-server requests no longer hit CORS pre-flight 401 errors
- Default HQL editor and Graph Visualiser queries changed from the invalid `V | RETURN *` to a valid HQL example (`QUERY getAll() => result <- N<People> RETURN result`)

**Vector / HNSW**
- Return `ZeroMagnitudeVector` error instead of dividing by zero on zero-magnitude input vectors
- Propagate non-`EntryPointNotFound` errors in `insert` instead of swallowing them
- `insert()` now rejects non-empty zero-magnitude vectors at the API boundary (both lmdb and rocks backends); empty placeholder vectors are still allowed
- `search_v` and `brute_force_search_v` now return a `GraphError` on negative `k` instead of panicking via `TryInto<usize>::unwrap()`; `brute_force_search_v` also replaces a `cosine_similarity().unwrap()` with a silent skip (`.ok()?`) for any zero-magnitude stored vector

**Value Arithmetic**
- Fix `I128 op I128` arithmetic: missing same-type arms caused `I128 + I128` to fall through to the cross-type signed arm and truncate to `I64`
- Promote cross-type signed integer arithmetic from `I64` to `I128` — `Value::I128(x) op Value::I8(y)` no longer silently truncates
- `abs()` now handles `Value::I128` (previously panicked)
- `is_zero()` guards in `Div` and `Rem` now detect `Value::I128(0)` (previously fell through to a `wrapping_div(0)` panic with no guard message)
- `min()` and `max()` cross-type integer pairs now use `Ord` comparison instead of `f64` promotion, preserving precision for values outside `f64`'s 53-bit mantissa

**CLI**
- `sparrow add <name>` now fails with a clear error in non-interactive mode when no instance name is given — previously silently used the project name, which was surprising in scripts

### Documentation

- `docs/HQL.md` — comprehensive HQL language reference (2 000+ lines, 21 sections):
  - Key concepts: `::` step separator, anonymous traversal `_`, identifier naming
  - Quick start with a complete social-network schema + query set
  - Schema definition: `N::`, `E::`, `V::`, `INDEX`, `UNIQUE INDEX`, `UNIQUE` edges, `DEFAULT` values, `vector(N)` node fields, schema versioning
  - Query definitions: `QUERY`, typed and optional parameters, `#[mcp]` and `#[model()]` macros
  - Node operations: `N<T>` (by ID, by index), `AddN`, `UPDATE`, `UpsertN`, `DROP`
  - Edge operations: `E<T>`, `AddE`, `UpsertE`
  - Vector operations: `AddV`, `BatchAddV`, `SearchV`, `SearchN`, `SearchBM25`, `Embed()`, `UpsertV`
  - Graph traversal: `Out`, `In`, `OutE`, `InE`, `FromN`, `ToN`, `FromV`, `ToV`
  - Filtering: `WHERE`, `EXISTS` / `!EXISTS`, `AND`/`OR`, all comparison operators (`GT`, `GTE`, `LT`, `LTE`, `EQ`, `NEQ`, `CONTAINS`, `IS_IN`), `INTERSECT`
  - Aggregation and sorting: `COUNT`, `RANGE`, `ORDER<Asc|Desc>`, `FIRST`, `GROUP_BY`, `AGGREGATE_BY`
  - Shortest path: `ShortestPath` (BFS), `ShortestPathDijkstras`, `ShortestPathAStar`
  - Mathematical functions: all arithmetic, unary, trigonometric, aggregate functions and constants
  - Vector reranking: `RerankRRF`, `RerankMMR` (all distance metrics), chaining
  - Field remapping: `::{}`, `::!{}`, spread `..`, `ID` step, closure `|x|{}`
  - Loops: `FOR...IN`, destructuring, object access, nested loops
  - Return values: all forms — literals, tuples, arrays, objects, inline remapped expressions
  - Type reference: all scalar types, `ID`, `Date`, `NOW`, `vector(N)`, arrays, objects, literals
  - Migrations syntax
  - Appendix: parser notes for contributors (grammar rules, module layout, feature flags)

### Internal

**Sparrow Studio**
- `sparrow-studio-ci.yml` — runs `pnpm install && pnpm build` on every push that touches `packages/studio/**` or `crates/sparrow-studio/**`
- Studio router is generic over the axum `State` type so it can be merged into any stateful router without cloning

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
