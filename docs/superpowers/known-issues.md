# SparrowDB — Known Issues & Workarounds

This file tracks non-trivial bugs or behavioural quirks discovered during
development that have been worked around rather than fixed at the source.
Each entry records the root cause, the affected code path, and the chosen
workaround so future engineers understand why the code looks the way it does.

---

## 1. `PutFlags::APPEND` + non-monotonic v6 UUIDs causes `MDB_KEYEXIST`

**Status:** ✅ Fixed. `PutFlags::APPEND` replaced with plain `put()` in
`add_n.rs`, `add_e.rs`, and `upsert.rs`. The `sparrow-benches` workaround
(`seed_graph` bypassing `add_n`/`add_edge`) can remain as-is; it is now
merely a performance optimisation (pre-sorting IDs), not a correctness
requirement. Regression tests added in
`crates/sparrow-core/src/sparrow_engine/tests/capacity_optimization_tests.rs`.

### Symptom

Running `cargo bench -p sparrow-benches --features cpu` with a medium-sized
fixture (≥ ~500 nodes) panics inside `seed_graph` with:

```
thread 'main' panicked at crates/sparrow-benches/src/lib.rs:<line>:
add_n failed: StorageError("MDB_KEYEXIST: Key/data pair already exists")
```

After fixing node insertion (by adding `thread::sleep` between batches),
the edge insertion panics with the same error:

```
thread 'main' panicked at crates/sparrow-benches/src/lib.rs:<line>:
add_edge failed: StorageError("MDB_KEYEXIST: Key/data pair already exists")
```

### Root cause

`add_n` and `add_edge` (in `sparrow-core`) both write their primary records
to LMDB using `PutFlags::APPEND`:

```rust
// add_n.rs line ~69
self.storage.nodes_db.put_with_flags(self.txn, PutFlags::APPEND, &node.id, &bytes)

// add_e.rs line ~112
self.storage.edges_db.put_with_flags(self.txn, PutFlags::APPEND, &edge_key, &bytes)
```

`PutFlags::APPEND` tells LMDB that the caller guarantees the new key is
**strictly greater** than every key already in the database. If the key is
equal to or less than the last key, LMDB returns `MDB_KEYEXIST`.

Node and edge IDs are generated via `v6_uuid()`:

```rust
// utils/id.rs
pub fn v6_uuid() -> u128 {
    uuid::Uuid::now_v6(&[1, 2, 3, 4, 5, 6]).as_u128()
}
```

UUID v6 is timestamp-based. When thousands of UUIDs are generated in a tight
loop the OS clock may not advance between consecutive calls (on macOS the
`mach_absolute_time` resolution is typically 41 ns; the UUID v1/v6 timestamp
uses 100 ns ticks). Two calls within the same 100 ns tick produce the same
timestamp and therefore the same u128 value — or, after clock wrapping /
adjustment, a *smaller* value.

Both cases violate the APPEND contract.

### Failed workaround: sleep-based rate limiting

Adding `thread::sleep(Duration::from_micros(2))` every 10 nodes was enough
to keep node IDs monotonic on macOS but the edge IDs (inserted as a separate
loop immediately after) triggered the same error because no sleep separated
edge UUID generation either.

Extending the sleep (e.g., 10 µs per node) could work in principle but
would add ~100 ms setup time for 10 k nodes, is fragile on loaded CI
machines, and does not fix the underlying ordering issue.

### Chosen workaround

`seed_graph` now **bypasses `add_n` / `add_edge` entirely** and writes
directly to the LMDB databases using `put()` (no `APPEND` flag):

1. Pre-generate all `node_count` node IDs with `v6_uuid()`, sort them, dedup.
2. Write `Node` structs (via `to_bincode_bytes()`) to `storage.nodes_db`.
3. Pre-generate all edge IDs, sort them, dedup.
4. Write `Edge` structs to `storage.edges_db`.
5. Write the out/in-edge index entries to `storage.out_edges_db` /
   `storage.in_edges_db` using the public key-builder helpers
   (`out_edge_key`, `in_edge_key`, `pack_edge_data`).

Sorting before writing keeps the database in the expected ascending-key
layout; using `put()` instead of `put_with_flags(APPEND)` removes the
monotonicity pre-condition entirely, making the code correct regardless of
clock resolution.

### Permanent fix (applied)

`PutFlags::APPEND` was removed from the three write sites that used it for
primary-record insertion:

| File | Change |
|------|--------|
| `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_n.rs` | `put_with_flags(APPEND)` → `put()` (both `add_n` and `add_n_with_vectors`) |
| `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_e.rs` | `put_with_flags(APPEND)` → `put()` on `edges_db` |
| `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/upsert.rs` | `put_with_flags(APPEND)` → `put()` on both `nodes_db` and `edges_db` |

`APPEND_DUP` on the out/in-edge index databases is intentionally unchanged —
within a single key the values *are* written in ascending order (edge data is
`edge_id || node_id` and edge IDs are generated one at a time, so there is
never a second value for the same key in a single request). `APPEND_DUP`
can stay.

Regression tests (`test_add_n_succeeds_when_existing_key_is_higher` and
`test_add_edge_succeeds_when_existing_key_is_higher`) in
`crates/sparrow-core/src/sparrow_engine/tests/capacity_optimization_tests.rs`
deterministically verify that both operations succeed when a higher key
already exists in the database.

---

## 2. `sparrow check` copies `queries.rs` to wrong path and runs `cargo check` from wrong directory

**Status:** ✅ Fixed. Both path references in `check.rs` corrected from `sparrow-repo-copy/sparrow-container/` to `sparrow-repo-copy/crates/sparrow-container/`.

### Symptom

`sparrow check <instance>` exits with code 1 after printing "Queries compiled (N queries)":

```
✓ Queries compiled (53 queries)
No such file or directory (os error 2)
```

### Root cause

`commands/check.rs` hardcodes two paths that assume the sparrow-container crate lives directly under `sparrow-repo-copy/`, not under the workspace's `crates/` subdirectory:

```rust
// check.rs — both lines wrong
let cargo_check_src = instance_workspace.join("sparrow-repo-copy/sparrow-container/src");
let sparrow_container_dir = instance_workspace.join("sparrow-repo-copy/sparrow-container");
```

The actual workspace layout has the crate at `sparrow-repo-copy/crates/sparrow-container/`. The `fs::copy` call therefore fails with `ENOENT`, and the subsequent `cargo check` would have targeted a non-existent directory.

### Workaround (now removed)

Callers added `|| true` to suppress the non-zero exit and then manually copied `queries.rs` into the correct `crates/sparrow-container/src/` path before running `docker build`.

### Fix (applied)

```rust
let cargo_check_src = instance_workspace.join("sparrow-repo-copy/crates/sparrow-container/src");
let sparrow_container_dir = instance_workspace.join("sparrow-repo-copy/crates/sparrow-container");
```

---

## 3. Generated Dockerfile `COPY`s `queries.rs` to a path Cargo never reads

**Status:** ✅ Fixed. `docker.rs` Dockerfile template updated; `sparrow-container/` destination changed to `crates/sparrow-container/`.

### Symptom

The Docker image builds successfully but the compiled binary contains the stub/default schema instead of the project's actual queries. Any query that was meant to be compiled in is silently absent.

### Root cause

`docker.rs` generates a Dockerfile with:

```dockerfile
COPY sparrow-container/ ./sparrow-container/
```

The build context contains two relevant directories:
- `sparrow-repo-copy/` — the full SparrowDB workspace, copied first with `COPY sparrow-repo-copy/ ./`
- `sparrow-container/` — the instance's generated `queries.rs`

`COPY sparrow-container/ ./sparrow-container/` places the generated file at `/build/sparrow-container/src/queries.rs`. But `Cargo.toml` declares the crate as a workspace member at `crates/sparrow-container`, so Cargo builds from `/build/crates/sparrow-container/src/queries.rs` — the copy from `sparrow-repo-copy/`, which is the unchanged template.

### Workaround (now removed)

Before each `docker build`, the generated `queries.rs` was manually copied into `sparrow-repo-copy/crates/sparrow-container/src/queries.rs` so that `COPY sparrow-repo-copy/ ./` would carry it to the correct location.

### Fix (applied)

```dockerfile
COPY sparrow-container/ ./crates/sparrow-container/
```

This overlays the generated file directly onto the workspace crate path, consistent with where Cargo resolves it.

---

## 5. `COPY sparrow-container/` in chef stage busts `cargo chef cook` cache on every query change

**Status:** ✅ Fixed. `COPY sparrow-container/ ./crates/sparrow-container/` removed from the chef/planner stages; kept only in the builder stage where the source is actually compiled.

### Symptom

Every `docker build` after a schema or query change takes as long as a full cold build (~10 minutes), even though the Rust dependency tree hasn't changed. `cargo chef cook` reruns from scratch each time.

### Root cause

The generated Dockerfile had `COPY sparrow-container/ ./crates/sparrow-container/` in the **chef** stage (before `cargo chef prepare`):

```dockerfile
FROM chef AS chef
COPY sparrow-repo-copy/ ./
COPY sparrow-container/ ./crates/sparrow-container/  ← unnecessary here

FROM chef AS planner
RUN cargo chef prepare --recipe-path recipe.json --bin sparrow-container
```

`cargo chef prepare` reads only `Cargo.toml` and `Cargo.lock` to produce `recipe.json`. It does not need `queries.rs`. But because `queries.rs` is copied into the chef stage before `cargo chef prepare`, any change to `queries.rs` invalidates the planner image. Docker then treats the `COPY --from=planner recipe.json` in the builder stage as a cache miss — even if `recipe.json` is byte-for-byte identical — because the planner *image* changed. This cascades to `cargo chef cook` being uncacheable.

Additionally, `sparrow check` writes the compiled `queries.rs` back into `sparrow-repo-copy/crates/sparrow-container/src/`, so `COPY sparrow-repo-copy/ ./` is also invalidated on every check run.

### Fix (applied)

Remove `COPY sparrow-container/ ./crates/sparrow-container/` from the chef stage. The planner only needs the workspace `Cargo.toml`/`Cargo.lock` layout; `queries.rs` belongs only in the builder stage:

```dockerfile
FROM chef AS chef
COPY sparrow-repo-copy/ ./           # no queries.rs overlay here

FROM chef AS planner
RUN cargo chef prepare ...           # recipe.json stable across query changes

FROM chef AS builder
COPY --from=planner recipe.json .
RUN cargo chef cook ...              # now properly cached
COPY sparrow-repo-copy/ ./
COPY sparrow-container/ ./crates/sparrow-container/   # queries.rs only here
RUN cargo build ...
```

With this fix, `cargo chef cook` is only invalidated when `Cargo.lock` changes. Schema/query iterations reuse the dep layer and only recompile `sparrow-container` itself (~60s).

---

## 7. `sparrow check` permanently dirties `sparrow-repo-copy`, busting Docker build cache on every run

**Status:** ✅ Fixed. `check.rs` now snapshots the original files in `sparrow-repo-copy` before overwriting and restores them after `cargo check` completes (success or failure).

### Symptom

After any `sparrow check` run, the next `docker build` (or `sparrow build`) takes as long as if `sparrow-repo-copy` had changed — even when no Rust source, `Cargo.toml`, or `Cargo.lock` has changed. The `COPY sparrow-repo-copy/ ./` layer in the builder stage never hits the Docker cache.

### Root cause

Step 5 of `check_instance` copies the generated `queries.rs` (and `config.hx.json`) into `sparrow-repo-copy/crates/sparrow-container/src/` so `cargo check` can resolve the workspace:

```rust
fs::copy(generated_src.join("queries.rs"), cargo_check_src.join("queries.rs"))?;
fs::copy(generated_src.join("config.hx.json"), cargo_check_src.join("config.hx.json"))?;
```

These files were never restored afterwards. On the next `docker build`:

```dockerfile
COPY sparrow-repo-copy/ ./       # now includes generated queries.rs → cache miss
COPY sparrow-container/ ./crates/sparrow-container/  # overlays correct file
```

The `COPY sparrow-repo-copy/ ./` layer hash changes (because `queries.rs` in `sparrow-repo-copy` changed), so Docker discards the cache and re-runs every subsequent step — including `cargo build`, which takes minutes.

Note: issue 5 removed `queries.rs` from the *chef* stage, which fixed the `cargo chef cook` cache. But the builder stage's `COPY sparrow-repo-copy/ ./` was still busted by the dirty `sparrow-repo-copy`.

### Fix (applied)

Snapshot the originals before overwriting; restore them unconditionally after `cargo check` completes — whether it succeeds, fails, or errors during spawn:

```rust
// Snapshot
let original_queries = fs::read(cargo_check_src.join("queries.rs")).ok();
let original_config  = fs::read(cargo_check_src.join("config.hx.json")).ok();

// Overwrite for cargo check
fs::copy(generated_src.join("queries.rs"),     cargo_check_src.join("queries.rs"))?;
fs::copy(generated_src.join("config.hx.json"), cargo_check_src.join("config.hx.json"))?;

// Run without propagating yet
let cargo_result = run_cargo_check(&sparrow_container_dir).await;

// Restore originals unconditionally
match original_queries {
    Some(c) => { let _ = fs::write(cargo_check_src.join("queries.rs"), &c); }
    None    => { let _ = fs::remove_file(cargo_check_src.join("queries.rs")); }
}
match original_config {
    Some(c) => { let _ = fs::write(cargo_check_src.join("config.hx.json"), &c); }
    None    => { let _ = fs::remove_file(cargo_check_src.join("config.hx.json")); }
}

let cargo_output = cargo_result?; // propagate after restore
```

The failure-path `generated_rust` read was also updated to read from `generated_src` (the instance's own workspace) rather than `cargo_check_src`, since the original has already been restored there by the time the failure branch runs.

---

## 8. `sparrow check` recompiles all dependencies from scratch on every run

**Status:** ✅ Fixed. `run_cargo_check` now passes `--target-dir .sparrow/check-cache` so compiled dependency artifacts are shared and reused across all `sparrow check` invocations.

### Symptom

`sparrow check` takes several minutes even after the first successful run, with `cargo check` rebuilding the full dependency tree each time. Iterating on schema changes (which only touch `queries.rs`) should be fast but is not.

### Root cause

`cargo check` ran with its working directory inside `sparrow-repo-copy/crates/sparrow-container` and no explicit `--target-dir`. Cargo therefore placed build artifacts in `sparrow-repo-copy/target/` (the workspace root). This directory is:

- **Ephemeral**: `sparrow-repo-copy` is managed by `ensure_sparrow_repo_cached()` and can be refreshed at any time, discarding the compiled artifacts.
- **Not shared**: each instance has its own `sparrow-repo-copy/` path, so even if the cache survived, two instances never share it.

Because `cargo check` was run on the same machine with the same toolchain and the same `Cargo.lock`, all this recompilation was wasted work.

### Fix (applied)

Pass `--target-dir` pointing to a project-level persistent cache directory:

```rust
let check_target_dir = project.root.join(".sparrow/check-cache");

Command::new("cargo")
    .arg("check")
    .arg("--color=never")
    .arg("--target-dir")
    .arg(&check_target_dir)
    .current_dir(&sparrow_container_dir)
    .output()
    .await
```

`.sparrow/check-cache` is:
- **Persistent**: lives outside `sparrow-repo-copy`, survives repo refreshes.
- **Shared**: all instances use the same cache; the dependency crates are identical across instances (same `Cargo.lock`), so their artifacts are valid for all of them.
- **Isolated from Docker builds**: entirely separate from the `sparrow-repo-copy/target/` that would be included in `COPY sparrow-repo-copy/ ./` (`.gitignore` and `.dockerignore` should already exclude `.sparrow/`).

First `sparrow check` after a toolchain or `Cargo.lock` change remains slow; every subsequent schema-only iteration is fast (only `sparrow-container` itself is rechecked, ~5 s).

### Design decision: why not dynamic linking?

Dynamic linking (`RUSTFLAGS="-C prefer-dynamic"`) was considered as an alternative speed-up on the grounds that `sparrow check` runs on the same local machine with the same toolchain — so the standard library and large crates could in principle be linked as `.dylib`/`.so` files rather than being statically compiled into each artifact.

**It does not apply to `cargo check`.** `cargo check` performs type-checking only; it produces `.rmeta` (metadata) files and no final binary. There is no linker invocation, so `-C prefer-dynamic` has nothing to accelerate. The expensive step is compiling dependency crates to `.rmeta` — which is exactly what `--target-dir` caching eliminates.

Dynamic linking *would* be relevant if a future `sparrow run` command compiled a local debug binary (bypassing Docker) and needed fast incremental link times. In that context, adding `RUSTFLAGS="-C prefer-dynamic"` to the local `cargo build` would cut link time from ~10 s to ~1 s on repeated builds. That is a separate command with a separate code path; it is not addressed here.

---

## 4. Generated `queries.rs` missing `UpsertAdapter` import — `upsert_n_with_defaults` unresolvable

**Status:** ✅ Fixed. `upsert::UpsertAdapter` added to the util import block in `sparrowc/generator/utils.rs`.

### Symptom

`cargo check` (and `cargo build`) on the generated `sparrow-container` crate fails with:

```
error[E0599]: no method named `upsert_n_with_defaults` found for struct
              `RwTraversalIterator<'db, 'arena, 'txn, I>` in the current scope
```

One error per `UpsertN` query in the project.

### Root cause

The code generator emits `upsert_n_with_defaults` calls in the Rust output for every `UpsertN` expression. `upsert_n_with_defaults` is a method on the `UpsertAdapter` trait, which is implemented for `RwTraversalIterator`. Trait methods are only callable when the trait is in scope.

The generated import block in `utils.rs` listed all other adapter traits but omitted `upsert::UpsertAdapter`:

```rust
util::{
    dedup::DedupAdapter, drop::Drop, exist::Exist,
    filter_ref::FilterRefAdapter, map::MapAdapter, ...
    aggregate::AggregateAdapter, group_by::GroupByAdapter, count::CountAdapter,
    // upsert::UpsertAdapter was missing
},
```

### Fix (applied)

```rust
util::{
    dedup::DedupAdapter, drop::Drop, exist::Exist,
    filter_ref::FilterRefAdapter, map::MapAdapter, ...
    aggregate::AggregateAdapter, group_by::GroupByAdapter, count::CountAdapter,
    upsert::UpsertAdapter,
},
```

---

## 6. `sparrow check` blocks Tokio thread pool — `std::process::Command` used in async context

**Status:** ✅ Fixed. `run_cargo_check` converted to `async fn`; `std::process::Command` replaced with `tokio::process::Command`.

### Symptom

`sparrow check <instance>` hangs the CLI for the full duration of `cargo check` (typically 30–120 s) without allowing other async tasks to make progress. Under load — e.g., when `check` is called concurrently for multiple instances — the Tokio runtime can fully stall.

### Root cause

`commands/check.rs` imported and used `std::process::Command`:

```rust
use std::process::Command;
// …
fn run_cargo_check(sparrow_container_dir: &Path) -> Result<CargoCheckOutput> {
    let output = Command::new("cargo")
        .arg("check")
        .current_dir(sparrow_container_dir)
        .output()       // ← blocking syscall — pins the Tokio thread
        .map_err(…)?;
```

`run_cargo_check` was a synchronous `fn` but was called directly inside the `async fn check_instance`:

```rust
async fn check_instance(…) -> Result<()> {
    // …
    let cargo_output = run_cargo_check(&sparrow_container_dir)?; // blocks!
```

`std::process::Command::output()` blocks the calling OS thread until the subprocess exits. In a Tokio context that thread is a worker in the async runtime; while it is blocked no other tasks on that thread can run.

### Fix (applied)

```rust
// check.rs
use tokio::process::Command; // replaces std::process::Command

async fn run_cargo_check(sparrow_container_dir: &Path) -> Result<CargoCheckOutput> {
    let output = Command::new("cargo")
        .arg("check")
        .arg("--color=never")
        .current_dir(sparrow_container_dir)
        .output()
        .await  // yields to the runtime while waiting
        .map_err(|e| eyre::eyre!("Failed to run cargo check: {}", e))?;
    // …
}
```

Call site updated to `run_cargo_check(…).await?`.

## 7. `sparrow-repo-copy/` — full engine source bundled per project, no pre-built base image

**Status:** ⚠️ Architectural limitation. No fix applied; workaround documented. Upstream fix requires a published base Docker image.

### Symptom

Every project using the sparrow CLI maintains a full clone of the SparrowDB Rust source (~342 MB) at `.sparrow/<instance>/sparrow-repo-copy/`. Docker build contexts transfer this entire tree on every build. `sparrow check` syncs it via `git pull` on every run. Any change to `queries.rs` can bust the entire Docker layer cache (see issue 5).

### Root cause

`sparrow build` compiles the `sparrow-container` binary from source inside Docker by copying the full engine source into the build context and running `cargo build`. There is no pre-built base image that has dependencies pre-compiled. Every project compiles the engine from scratch (or from Docker layer cache when it holds).

The `sparrow-repo-copy` remote is initialised by `create_git_cache` in `build.rs`, which clones from `SPARROW_REPO_URL = "https://github.com/helixdb/helix-db.git"`. In practice the remote can drift to an unrelated URL (observed: the project's own GitHub remote), causing `git pull` to pull the wrong codebase and `cargo check` to fail with stale engine source.

### Workaround (local dev)

Repoint the remote to the local SparrowDB checkout so `git pull` is a local filesystem operation and any local engine fixes are immediately available:

```bash
cd .sparrow/<instance>/sparrow-repo-copy
git remote set-url origin /Users/franciscobaptista/Development/SparrowDB
git pull
```

### Permanent fix (upstream, not yet applied)

Publish a pre-built Docker base image with SparrowDB dependencies already compiled. The generated Dockerfile would become:

```dockerfile
FROM sparrowdb/base:latest   # deps baked in, rarely changes
COPY queries.rs ./crates/sparrow-container/src/
RUN cargo build --release --package sparrow-container
```

This eliminates `sparrow-repo-copy/` entirely, reduces the build context from ~342 MB to a single file, and makes `cargo chef cook` obsolete for end-users.

---

## 8. HQL queries require a full compile-and-link cycle — no hot injection of schema changes

**Status:** 🚧 Architectural limitation. No fix applied. Options documented below.

### Problem

Every schema or query change in a project requires:

1. `sparrow check` — runs the HQL compiler, generates `queries.rs`, copies it into `sparrow-repo-copy/`, invokes `cargo check` (~30–120 s).
2. `sparrow build` — rebuilds the Docker image (~60–600 s, depending on cache).
3. Container restart — the old binary is replaced.

The root cause is that HQL queries are compiled to Rust source code at codegen time and linked into the `sparrow-container` binary. Changing a query requires producing a new binary. There is no runtime mechanism for loading or reloading query definitions without a rebuild.

This is tightly coupled to the `sparrow-repo-copy/` problem (issue 7): the full engine source must be present so that the freshly generated `queries.rs` can be compiled against it.

### Option A — Pre-built base image (incremental improvement, not hot injection)

Publish a versioned Docker base image with the SparrowDB engine dependencies pre-compiled (`cargo chef cook` output baked in). Projects `FROM` this image and only compile `sparrow-container` itself.

**Result:** Build time drops from ~600 s (cold) / ~90 s (cache hit) to ~60 s (only `sparrow-container` recompiled). `sparrow-repo-copy/` is eliminated. Still requires a Docker build + container restart per query change — not hot injection.

**Effort:** Publish CI pipeline for `sparrowdb/base:latest`. Modify `docker.rs` template. No engine changes.

**Trade-offs:**
- Base image must be republished whenever `Cargo.lock` changes.
- Projects must pin a base image tag to avoid silent ABI drift.
- `queries.rs` must still be compiled against the engine's Rust types — the generated code is tightly coupled to engine internals.

### Option B — Dynamic library (`.dylib` / `.so`) hot-swap

Compile each project's `queries.rs` as a shared library rather than linking it into the binary at build time. At startup (and on `SIGHUP` / a reload endpoint), `sparrow-container` `dlopen`s the current library and registers its query handlers.

**Result:** Schema changes require recompiling only the query library (~5–15 s). No Docker rebuild. No container restart. Hot path is: `sparrow check` → `cargo build --package queries-lib` → signal container → reload.

**Effort:** High. Requires:
- A stable ABI boundary between the engine and the query library (`extern "C"` or a vtable-based interface).
- Rust's `dylib` crate type with careful `no_mangle` exports.
- Runtime `dlopen`/`dlsym` in `sparrow-container` with safe unload semantics (outstanding requests must drain before the old library is freed).
- Platform portability concerns: `.dylib` on macOS, `.so` on Linux — symbol visibility differs.

**Trade-offs:**
- ABI instability: any change to engine types that the query library uses (node/edge structs, iterator types) requires a coordinated bump.
- Difficult to test — dlopen failures are runtime, not compile-time.
- LMDB handles and arena allocators held across the ABI boundary are extremely dangerous to unload.
- Likely not worth it unless the hot-reload loop is measured in seconds per day of active development.

### Option C — WASM query modules

Compile HQL directly to WebAssembly. `sparrow-container` embeds a WASM runtime (e.g. [Wasmtime](https://wasmtime.dev)) and loads the query module at startup or on demand.

**Result:** Query changes compile to WASM (~15–30 s) and are hot-swapped without restarting the container. The engine and query code are fully isolated — WASM sandbox prevents ABI drift crashes.

**Effort:** Very high. Requires:
- A new HQL → WASM compilation backend (the current backend emits Rust source).
- A WASM host ABI: the engine must expose graph traversal primitives (read/write transactions, node/edge access) as WASM imports; the query module calls them.
- Wasmtime or Wasmer integrated into `sparrow-container`.
- Serialisation boundary: data crossing the WASM boundary must be encoded (likely as `bincode` or `flatbuffers`); this adds overhead on every graph operation.

**Trade-offs:**
- Best isolation and portability of the three options.
- Performance: WASM JIT is fast, but the host–guest serialisation boundary on hot paths (per-node iteration) could dominate latency.
- Longest implementation path — effectively a second compiler backend.

### Option D — Runtime HQL interpretation (remove the compile step entirely)

The HQL compiler (`sparrowc`) already exists inside `sparrow-core` as a library. Instead of generating `queries.rs` at deploy time, `sparrow-container` bundles `sparrowc` and compiles + executes HQL at request time (or at startup from `.hx` files mounted into the container).

**Result:** No `cargo check`, no Docker rebuild, no `sparrow-repo-copy/`. Schema changes take effect as soon as the new `.hx` files are mounted and the container is signalled (or on the next request, if hot-reloaded from disk).

**Effort:** Medium–high. Requires:
- `sparrowc` to produce an in-memory AST/IR rather than Rust source text.
- A query executor that walks the IR and dispatches to engine primitives — essentially an interpreter loop over the existing traversal operators.
- The traversal operators (`add_n`, `add_e`, `upsert`, `get_n`, etc.) are already callable as Rust functions; the interpreter glues them together dynamically instead of at compile time.

**Trade-offs:**
- Eliminates the entire code-generation pipeline and `sparrow-repo-copy/`.
- Performance: interpreted dispatch adds overhead compared to inlined Rust, but for graph queries the dominant cost is storage I/O, not dispatch. Initial benchmarking is needed.
- Type checking: the current compiler catches type errors at `cargo check` time. An interpreter must either check types at parse time (when `.hx` files are loaded) or at execution time. Parse-time checking is preferable and feasible given the existing type-checker in `sparrowc`.
- Smallest surface area of the four options; no new dependencies, no ABI design, no WASM runtime.

### Recommended path

**Short term:** Option A (pre-built base image) — eliminates `sparrow-repo-copy/`, cuts build time to ~60 s, unblocks users without engine changes.

**Long term:** Option D (runtime interpretation) — removes the compile loop entirely, enables live schema iteration, and aligns with how graph databases (Neo4j Cypher, Gremlin) handle query execution. The existing `sparrowc` infrastructure is the foundation; the main work is an IR executor rather than a Rust code emitter.
