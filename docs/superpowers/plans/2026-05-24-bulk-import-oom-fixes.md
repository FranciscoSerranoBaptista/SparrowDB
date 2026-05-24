# Bulk Import OOM & Stability Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix all issues from the 2026-05-24 forensic bug report: OOM during bulk import of 43K records, Docker daemon instability, CLI import UX issues, and add OrbStack support.

**Architecture:** Six independent fixes across the CLI (`sparrow-cli`), storage engine (`sparrow-core`), and compiler (`sparrowc`). The critical fix is a compiler optimization that replaces O(n) full-table scans with O(log n) index lookups when WHERE clauses filter on indexed fields. Supporting fixes add periodic LMDB sync, bounded memory defaults, CLI resilience, and OrbStack container runtime support.

**Tech Stack:** Rust, heed3 (LMDB wrapper), axum, reqwest, clap, tokio, indicatif

---

### Task 1: Add OrbStack Container Runtime Support

OrbStack is a Docker Desktop replacement for macOS with better stability and resource management. It uses the `docker` binary (drop-in compatible) but is launched via `OrbStack.app`.

**Files:**
- Modify: `crates/sparrow-cli/src/config.rs:55-81`
- Modify: `crates/sparrow-cli/src/docker.rs:220-382`

- [ ] **Step 1: Add OrbStack variant to ContainerRuntime enum**

```rust
// crates/sparrow-cli/src/config.rs — modify the ContainerRuntime enum

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContainerRuntime {
    #[default]
    Docker,
    Podman,
    OrbStack,
}

impl ContainerRuntime {
    pub fn binary(&self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::OrbStack => "docker", // OrbStack provides its own docker binary
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Docker => "DOCKER",
            Self::Podman => "PODMAN",
            Self::OrbStack => "ORBSTACK",
        }
    }
}
```

- [ ] **Step 2: Add OrbStack startup in start_runtime_daemon**

In `crates/sparrow-cli/src/docker.rs`, add the OrbStack case to `start_runtime_daemon`:

```rust
// Inside the match (runtime, platform) block, add before the catch-all:

// OrbStack on macOS
(ContainerRuntime::OrbStack, "macos") => {
    Step::verbose_substep("Starting OrbStack for macOS...");
    Command::new("open")
        .args(["-a", "OrbStack"])
        .output()
        .map_err(|e| eyre!("Failed to start OrbStack: {}", e))?;
}

// OrbStack on Linux (native)
(ContainerRuntime::OrbStack, "linux") => {
    Step::verbose_substep("Starting OrbStack on Linux...");
    let result = Command::new("orbctl").args(["start"]).output();
    match result {
        Ok(output) if output.status.success() => {}
        _ => {
            return Err(eyre!(
                "Failed to start OrbStack. Is it installed? See https://orbstack.dev"
            ));
        }
    }
}
```

- [ ] **Step 3: Build and verify compilation**

Run: `cargo build --package sparrow-cli 2>&1 | tail -5`
Expected: Compilation succeeds. Any `match` exhaustiveness errors should be fixed by adding `ContainerRuntime::OrbStack` arms to existing matches — grep for `ContainerRuntime::Docker` to find them all.

- [ ] **Step 4: Commit**

```bash
git add crates/sparrow-cli/src/config.rs crates/sparrow-cli/src/docker.rs
git commit -m "feat(cli): add OrbStack as container runtime option"
```

---

### Task 2: CLI Import — HTTP Timeout, Signal Handler, .ndjson Support

Three independent improvements to `sparrow import` that improve resilience during bulk imports.

**Files:**
- Modify: `crates/sparrow-cli/src/commands/import.rs:51-76,194-210,273-416`

- [ ] **Step 1: Add .ndjson extension recognition**

In `detect_format`, add `"ndjson"` to the JSON match arm:

```rust
// crates/sparrow-cli/src/commands/import.rs — in detect_format()

match ext.as_str() {
    "json" | "jsonl" | "ndjson" => Ok(ImportFormat::Json),
    "csv" | "tsv" => Ok(ImportFormat::Csv),
    "parquet" | "pq" => Ok(ImportFormat::Parquet),
    other => bail!(
        "cannot infer format from extension '.{}' — use --format json|csv|parquet",
        other
    ),
}
```

- [ ] **Step 2: Add test for .ndjson detection**

```rust
// At the end of the detect_format_by_extension test:

assert_eq!(detect_format(Path::new("a.ndjson"), None).unwrap(), ImportFormat::Json);
assert_eq!(detect_format(Path::new("a.jsonl"), None).unwrap(), ImportFormat::Json);
```

- [ ] **Step 3: Run the test**

Run: `cargo test --package sparrow-cli -- detect_format_by_extension -v`
Expected: PASS

- [ ] **Step 4: Add HTTP timeout to build_client**

```rust
// crates/sparrow-cli/src/commands/import.rs — modify build_client()

fn build_client(token: Option<&str>) -> Result<Client> {
    let mut builder = Client::builder()
        .pool_max_idle_per_host(128)
        .tcp_nodelay(true)
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10));

    if let Some(tok) = token {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            "x-api-key",
            header::HeaderValue::from_str(tok)
                .map_err(|_| eyre::eyre!("auth token contains invalid header characters"))?,
        );
        builder = builder.default_headers(headers);
    }

    builder.build().map_err(|e| eyre::eyre!("building HTTP client: {e}"))
}
```

- [ ] **Step 5: Add Ctrl+C handler for graceful summary**

In the `run` function, after creating the `ok_count`/`err_count`/`aborted` Arcs and before the `stream::iter(...)` call, add a Ctrl+C handler:

```rust
// crates/sparrow-cli/src/commands/import.rs — inside run(), after the aborted Arc

// Install Ctrl+C handler to print summary on interrupt
{
    let ok_count = Arc::clone(&ok_count);
    let err_count = Arc::clone(&err_count);
    let aborted = Arc::clone(&aborted);
    let start = start.clone();
    let pb = Arc::clone(&pb);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            aborted.store(true, Ordering::Relaxed);
            pb.finish_and_clear();
            let elapsed = start.elapsed();
            let ok = ok_count.load(Ordering::Relaxed);
            let err = err_count.load(Ordering::Relaxed);
            eprintln!(
                "\nInterrupted after {:.1}s — {} ok, {} failed, {} pending",
                elapsed.as_secs_f64(),
                ok,
                err,
                total as u64 - ok - err
            );
            std::process::exit(130);
        }
    });
}
```

Note: `Instant` does not implement `Clone`. Use a separate `let start_for_handler = Instant::now();` before the stream, captured by the handler. Or capture `start` as a `u64` timestamp. The simplest approach: move the handler spawn to after `let start = Instant::now();` and make `start_for_handler` a copy of the elapsed reference via a shared `Arc<Instant>` or just re-instantiate. Since `Instant` is `Copy`, just `let start_copy = start;`.

- [ ] **Step 6: Build and verify**

Run: `cargo build --package sparrow-cli 2>&1 | tail -5`
Expected: Compiles cleanly

- [ ] **Step 7: Commit**

```bash
git add crates/sparrow-cli/src/commands/import.rs
git commit -m "fix(cli): add HTTP timeout, signal handler, and .ndjson support to import"
```

---

### Task 3: Reduce Default db_max_size_gb

The current default of 20GB for LMDB's virtual address space reservation is excessive for containers with 2GB memory limits. While this is virtual (not physical), it affects page fault behavior under memory pressure.

**Files:**
- Modify: `crates/sparrow-cli/src/config.rs:201`
- Modify: `crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs:104-108`

- [ ] **Step 1: Change CLI default from 20 to 4**

```rust
// crates/sparrow-cli/src/config.rs:201

fn default_db_max_size_gb() -> u32 { 4 }
```

- [ ] **Step 2: Change core fallback from 100 to 4**

```rust
// crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs:104-108

let db_size = if config.db_max_size_gb.unwrap_or(4) >= 9999 {
    9998
} else {
    config.db_max_size_gb.unwrap_or(4)
};
```

- [ ] **Step 3: Build and test**

Run: `cargo build --workspace 2>&1 | tail -5`
Expected: Compiles cleanly

- [ ] **Step 4: Commit**

```bash
git add crates/sparrow-cli/src/config.rs crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs
git commit -m "fix(storage): reduce default db_max_size_gb from 20 to 4

Reduces LMDB virtual address space reservation to 4GB by default.
The previous 20GB default caused excessive page-fault pressure in
containers with 2GB memory limits during bulk imports."
```

---

### Task 4: Add Periodic force_sync in Writer Thread

During bulk imports, LMDB dirty pages accumulate because sync only happens implicitly on `wtxn.commit()`. Adding explicit `force_sync()` every N write transactions flushes dirty pages and bounds RSS growth.

**Files:**
- Modify: `crates/sparrow-core/src/sparrow_gateway/worker_pool/mod.rs:279-332`

- [ ] **Step 1: Add force_sync call after every 500 write transactions**

In `Worker::start_writer`, add a write counter and periodic sync:

```rust
// crates/sparrow-core/src/sparrow_gateway/worker_pool/mod.rs
// In start_writer(), modify the loop body:

pub fn start_writer(
    rx: Receiver<ReqMsg>,
    graph_access: Arc<SparrowGraphEngine>,
    router: Arc<SparrowRouter>,
    io_rt: Arc<Runtime>,
) -> Worker {
    let handle = std::thread::spawn(move || {
        sparrow_metrics::init_thread_local();
        let _io_guard = io_rt.enter();

        let mut write_count: u64 = 0;
        const SYNC_INTERVAL: u64 = 500;

        loop {
            match rx.recv() {
                Ok((req, ret_chan)) => {
                    let (cont_tx, cont_rx) = flume::bounded::<ContMsg>(1);

                    request_mapper(
                        req,
                        ret_chan,
                        graph_access.clone(),
                        &router,
                        &io_rt,
                        &cont_tx,
                    );

                    drop(cont_tx);

                    while let Ok((ret_chan, cfn)) = cont_rx.recv() {
                        let result = cfn().map_err(Into::into);
                        if ret_chan.send(result).is_err() {
                            trace!(
                                "Client disconnected before continuation response could be sent"
                            );
                        }
                    }

                    write_count += 1;
                    if write_count % SYNC_INTERVAL == 0 {
                        if let Err(e) = graph_access.storage.graph_env.force_sync() {
                            error!("LMDB force_sync failed after {} writes: {e}", write_count);
                        } else {
                            trace!("LMDB force_sync after {} writes", write_count);
                        }
                    }
                }
                Err(_) => {
                    trace!("Writer request channel was dropped, shutting down");
                    // Final sync before shutdown
                    if let Err(e) = graph_access.storage.graph_env.force_sync() {
                        error!("LMDB final force_sync failed: {e}");
                    }
                    break;
                }
            }
        }
    });
    Worker { _handle: handle }
}
```

- [ ] **Step 2: Build and verify**

Run: `cargo build --package sparrow-core --features lmdb 2>&1 | tail -10`
Expected: Compiles cleanly. If `force_sync()` has a different signature in heed3 0.22, check the docs — it's `pub fn force_sync(&self) -> Result<()>`.

- [ ] **Step 3: Commit**

```bash
git add crates/sparrow-core/src/sparrow_gateway/worker_pool/mod.rs
git commit -m "fix(storage): add periodic LMDB force_sync every 500 writes

Bounds RSS growth during bulk imports by flushing dirty pages to disk
periodically. Without this, dirty pages accumulated across all 43K
transactions and exhausted the container's 2GB memory limit."
```

---

### Task 5: Add LMDB Stale Lock Cleanup on Startup

When the OOM killer sends SIGKILL to SparrowDB, LMDB's lock file (`data.mdb-lock`) is left in a locked state. On restart, this can cause failures or compound crash-loop I/O pressure.

**Files:**
- Modify: `crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs:97-116`

- [ ] **Step 1: Add stale lock detection before LMDB env open**

Insert lock file cleanup logic before the `EnvOpenOptions::new()` call:

```rust
// crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs
// Inside SparrowGraphStorage::new(), after fs::create_dir_all(path):

fs::create_dir_all(path)?;

// Clean up stale LMDB lock file from unclean shutdown (e.g., OOM kill).
// LMDB re-creates the lock file on env open, so removing a stale one is safe.
let lock_path = std::path::Path::new(path).join("data.mdb-lock");
let data_path = std::path::Path::new(path).join("data.mdb");
if lock_path.exists() && !data_path.exists() {
    // Lock file without a data file is always stale
    tracing::warn!("Removing orphaned LMDB lock file: {}", lock_path.display());
    let _ = fs::remove_file(&lock_path);
} else if lock_path.exists() {
    tracing::info!(
        "LMDB lock file present at {} — normal if previous shutdown was clean",
        lock_path.display()
    );
}
```

Note: LMDB handles stale locks internally on most platforms (it checks the PID in the lock file). This code only removes truly orphaned locks (lock without data file) and logs the presence of lock files for debugging crash-loop scenarios. Do NOT unconditionally delete the lock file when the data file exists — LMDB's internal recovery handles that case.

- [ ] **Step 2: Build and test**

Run: `cargo build --package sparrow-core --features lmdb 2>&1 | tail -5`
Expected: Compiles cleanly

- [ ] **Step 3: Run storage tests**

Run: `cargo test --package sparrow-core --features lmdb -- --test-threads=1 2>&1 | tail -20`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs
git commit -m "fix(storage): detect and clean up stale LMDB lock files on startup

After an OOM kill, the lock file is left behind. This logs its presence
for debugging and removes truly orphaned lock files (lock without data)."
```

---

### Task 6: Compiler — Optimize WHERE+EQ on Indexed Field to Use NFromIndex

This is the critical performance fix. Currently, `N<Type>::WHERE(_::{field}::EQ(value))` compiles to a full O(n) table scan + filter, even when `field` has a secondary index. This causes O(n^2) total cost for bulk upserts (43K records = ~878 million comparisons).

The fix: after a WHERE step is generated, check if it's a simple equality check on an indexed field. If so, replace the `NFromType` source step with `NFromIndex` and remove the WHERE step.

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/analyzer/methods/traversal_validation.rs:948-1031`

**Design constraints:**
- The optimization must only trigger when ALL of these conditions hold:
  1. `gen_traversal.source_step` is `NFromType` (a full type scan, not already indexed)
  2. The WHERE expression is `_::{field}::EQ(value)` — anonymous traversal, single property access, single EQ comparison
  3. The property `field` has a secondary index (check `ctx.node_fields` for `is_indexed()`)
- The type returned (`cur_ty`) must NOT change — it stays `Type::Nodes(Some(...))` 
- `gen_traversal.should_collect` must NOT change — keep whatever was set by NFromType
- `gen_traversal.traversal_type` stays `TraversalType::Ref`

- [ ] **Step 1: Add the optimization function**

Add this function before `validate_traversal` in `traversal_validation.rs`:

```rust
/// Checks if a WHERE step that was just added can be replaced with an index lookup.
///
/// Pattern: `N<Type>::WHERE(_::{field}::EQ(value))` where `field` is @indexed or @unique.
///
/// When detected, replaces the NFromType source step with NFromIndex and removes
/// the WHERE step, converting an O(n) scan + filter into an O(log n) index lookup.
fn try_optimize_where_to_index(
    ctx: &Ctx<'_>,
    gen_traversal: &mut GeneratedTraversal,
    node_type: &str,
) -> bool {
    // 1. Source must be NFromType
    let source_label = match &gen_traversal.source_step {
        Separator::Period(SourceStep::NFromType(nft)) => nft.label.clone(),
        _ => return false,
    };

    // 2. Last step must be a WHERE
    let where_step = match gen_traversal.steps.last() {
        Some(sep) => match sep.inner() {
            GeneratedStep::Where(w) => w,
            _ => return false,
        },
        None => return false,
    };

    // 3. WHERE must contain a BoExp::Expr (a traversal expression)
    let Where::Ref(wr) = where_step;
    let traversal = match &wr.expr {
        BoExp::Expr(tr) => tr,
        _ => return false,
    };

    // 4. Traversal must have exactly 2 steps: PropertyFetch + BoolOp::Eq
    if traversal.steps.len() != 2 {
        return false;
    }

    let prop_name = match traversal.steps[0].inner() {
        GeneratedStep::PropertyFetch(p) => p,
        _ => return false,
    };

    let eq_value = match traversal.steps[1].inner() {
        GeneratedStep::BoolOp(BoolOp::Eq(eq)) => &eq.right,
        _ => return false,
    };

    // 5. Property must be indexed on this node type
    let prop_name_str = match prop_name {
        GenRef::Literal(s) | GenRef::Std(s) | GenRef::Ref(s) => s.clone(),
        _ => return false,
    };

    let node_fields = match ctx.node_fields.get(node_type) {
        Some(fields) => fields,
        None => return false,
    };

    let field_def = match node_fields.get(prop_name_str.as_str()) {
        Some(f) => f,
        None => return false,
    };

    if !field_def.is_indexed() {
        return false;
    }

    // All conditions met — rewrite source to NFromIndex and drop the WHERE step
    gen_traversal.source_step = Separator::Period(SourceStep::NFromIndex(NFromIndex {
        label: source_label,
        index: GenRef::Literal(prop_name_str),
        key: eq_value.clone(),
    }));

    gen_traversal.steps.pop(); // Remove the WHERE step

    true
}
```

- [ ] **Step 2: Call the optimization after WHERE step generation**

In `validate_traversal`, inside the `StepType::Where(expr)` arm, after the WHERE step has been pushed to `gen_traversal.steps` (after line ~1018), add the optimization call:

```rust
// After the WHERE step is added to gen_traversal.steps, try to optimize it.
// This must come after the step is fully validated and generated.
if let Type::Nodes(Some(ref nt)) = cur_ty {
    if try_optimize_where_to_index(ctx, gen_traversal, nt) {
        tracing::trace!("Optimized WHERE to index lookup for N<{}>", nt);
    }
}
```

There are three places where a WHERE step gets added (lines ~983-988, ~1014-1018, and the BoExp::Exists arms don't add a normal WHERE). The optimization call should go after each of the two places that push a `GeneratedStep::Where`. Add it after the closing brace of the `GeneratedStatement::Traversal` branch AND after the closing brace of the `GeneratedStatement::BoExp` branch, before the `_ =>` unreachable branch.

The cleanest approach: add the optimization call ONCE, after the entire `match stmt { ... }` block completes (around line 1030), just before the end of the `StepType::Where` arm.

- [ ] **Step 3: Add required imports**

At the top of `traversal_validation.rs`, ensure these are imported (most already are):

```rust
use crate::sparrowc::generator::source_steps::NFromIndex;
use crate::sparrowc::generator::traversal_steps::{Step as GeneratedStep, Where};
use crate::sparrowc::generator::bool_ops::BoolOp;
```

Check that `NFromIndex` and `BoolOp` are in scope. `NFromIndex` is already imported at line 28. `BoolOp` should already be imported via the existing imports.

- [ ] **Step 4: Build and verify**

Run: `cargo build --package sparrow-core --features lmdb 2>&1 | tail -20`
Expected: Compiles cleanly. Fix any type mismatches — the key types to check:
- `GenRef<String>` for the property name — verify it matches what `PropertyFetch` stores
- `GeneratedValue` for `eq.right` — verify it matches `NFromIndex.key`'s type
- `field_def.is_indexed()` — verify this method exists on the field type in `ctx.node_fields`

- [ ] **Step 5: Run compiler tests**

Run: `cargo test --package sparrow-core --features lmdb -- --test-threads=1 2>&1 | tail -30`
Expected: All existing tests pass. The optimization is transparent — it produces the same results, just faster.

- [ ] **Step 6: Run HQL integration tests**

Run: `cargo test --package hql-tests --features lmdb 2>&1 | tail -30`
Expected: All pass. If any WHERE-based tests fail, the optimization may have changed the codegen output for tests that check exact generated code — inspect the diff.

- [ ] **Step 7: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/analyzer/methods/traversal_validation.rs
git commit -m "perf(compiler): optimize WHERE+EQ on indexed fields to use NFromIndex

When N<Type>::WHERE(_::{field}::EQ(value)) targets an @indexed or
@unique field, the compiler now emits an O(log n) B-tree index lookup
instead of an O(n) full table scan + filter.

For bulk imports of 43K records, this reduces total comparisons from
~878 million (O(n^2)) to ~700K (O(n log n)), eliminating the primary
driver of OOM during bulk upserts."
```

---

## Verification

After all tasks are complete, run the full workspace test suite:

```bash
cargo test --workspace --features lmdb,server 2>&1 | tail -30
```

All tests must pass. Then verify the build:

```bash
cargo build --package sparrow-container --features lmdb --release 2>&1 | tail -5
cargo build --package sparrow-cli 2>&1 | tail -5
```
