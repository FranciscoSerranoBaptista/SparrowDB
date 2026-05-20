# SparrowDB — Project 0: Critical Bug Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix four crash/freeze/algorithmic-decay bugs that affect production stability. No new features, no refactoring beyond what is needed to fix the specific defect.

**Architecture:** All four bugs are self-contained — no task depends on another. They touch `bm25/`, `vector_core/`, `mcp/`, and `embedding_providers/`. Tasks may be executed in parallel once the plan is understood.

**Tech Stack:** Rust, heed3 (LMDB), bumpalo, tokio, axum

**Prerequisite:** These tasks touch `vector_core/lmdb/vector_core.rs`. If Project 1 (vector stability) Task 1 has already been applied to that file, pull the latest before starting Task 2 here.

**Constraint:** NEVER run `cargo install` or `cargo build --release`. Only `cargo check` and `cargo test`.

---

## What this plan does NOT include

- `Loc.span: String` memory amplification — this code is deleted wholesale in Project 2 (Chumsky rewrite). Not worth refactoring now.
- Value arithmetic overflow — already in Project 1 Task 3.
- Any new HTTP endpoints.

---

## File Structure

| File | Change |
|---|---|
| `sparrow-db/src/sparrow_engine/bm25/lmdb_bm25.rs` | Guard `avgdl=0, doc_len=0` NaN in `calculate_bm25_score` |
| `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` | Add degree-bound pruning of back-links in `set_neighbours` |
| `sparrow-db/src/sparrow_gateway/mcp/mcp.rs` | Cache materialized results in `MCPConnection` to fix O(N²) pagination |
| `sparrow-db/src/sparrow_gateway/embedding_providers/mod.rs` | Remove `block_on` from `fetch_embedding` sync path |
| `sparrow-db/src/sparrow_gateway/mcp/mcp.rs` | Update embedding call sites to use async path |

---

## Task 1: Fix BM25 NaN Poisoning

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/bm25/lmdb_bm25.rs`
- Test: same file, `#[cfg(test)]` block

**Background:** In `calculate_bm25_score` (line 831), when `avgdl == 0.0` (no documents yet), the fallback is `doc_len as f64`. If `doc_len` is also 0 (empty document inserted as first doc), `avgdl` becomes `0.0`. Line 854 then computes `doc_len.abs() / avgdl = 0.0 / 0.0 = NaN`. NaN propagates into BinaryHeap score comparisons, causing undefined sort order and potential panics throughout the text search subsystem.

- [ ] **Step 1: Write the failing test**

In the `#[cfg(test)]` module in `lmdb_bm25.rs`:

```rust
#[test]
fn test_bm25_score_no_nan_when_avgdl_and_doc_len_are_zero() {
    // Simulate the worst case: empty document is the only document in the index.
    // avgdl = 0.0, doc_len = 0, tf = 1, df = 1, total_docs = 1
    // Before fix: 0.0 / 0.0 = NaN poisons the heap.
    // After fix: score must be a finite f32.
    let config = HBM25Config::default(); // or however it's constructed; check existing tests
    let bm25_core = /* minimal VectorCore or standalone score function */;
    let score = bm25_core.calculate_bm25_score(
        1,  // tf
        0,  // doc_len (empty document)
        1,  // df
        1,  // total_docs
        0.0, // avgdl (no docs seen before)
    );
    assert!(score.is_finite(), "BM25 score must not be NaN or infinite: got {score}");
}
```

> **Note:** If `calculate_bm25_score` is private, test it via a public method. Look for an existing test that calls `search()` and replicate the pattern, inserting an empty document first.

- [ ] **Step 2: Run to verify it fails**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test test_bm25_score_no_nan_when_avgdl_and_doc_len_are_zero 2>&1 | tail -15
```

Expected: FAIL (NaN or panic).

- [ ] **Step 3: Fix `calculate_bm25_score`**

In `sparrow-db/src/sparrow_engine/bm25/lmdb_bm25.rs`, replace lines 847–854:

```rust
// Before:
let avgdl = if avgdl > 0.0 { avgdl } else { doc_len as f64 };
let tf_component = (tf * (self.k1 + 1.0))
    / (tf + self.k1 * (1.0 - self.b + self.b * (doc_len.abs() / avgdl)));
```

```rust
// After:
// Guard: if both avgdl and doc_len are 0 (empty doc, empty corpus), treat doc_len/avgdl as 1.0.
// This avoids 0/0 = NaN. A zero-length document in an empty corpus contributes no term frequency
// signal; returning a finite (low) score is strictly better than NaN poisoning the heap.
let length_ratio = if avgdl > 0.0 {
    doc_len / avgdl
} else if doc_len > 0.0 {
    1.0 // non-empty doc, no prior avgdl — treat as normalized
} else {
    1.0 // empty doc, empty corpus — neutral ratio
};
let tf_component = (tf * (self.k1 + 1.0))
    / (tf + self.k1 * (1.0 - self.b + self.b * length_ratio));
```

- [ ] **Step 4: Run the test**

```bash
cargo test test_bm25_score_no_nan_when_avgdl_and_doc_len_are_zero 2>&1 | tail -10
cargo check --workspace 2>&1 | tail -5
```

Expected: test passes, no compile errors.

- [ ] **Step 5: Commit**

```bash
git add sparrow-db/src/sparrow_engine/bm25/lmdb_bm25.rs
git commit -m "fix(bm25): guard 0/0 NaN in calculate_bm25_score when empty doc is first insertion"
```

---

## Task 2: Fix HNSW Hub-Node Degree Unbounded Back-Links

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` (`set_neighbours`)
- Test: `sparrow-db/src/sparrow_engine/tests/traversal_tests/vector_traversal_tests.rs`

**Background:** In `set_neighbours`, adding a back-link `[neighbor_id | level | id]` to the neighbor's edge list never checks whether `neighbor_id` already has `m_max_0` edges at level 0 (or `m` at higher levels). The standard HNSW algorithm (Malkov & Yashunin, Algorithm 1, line 14) explicitly prunes each neighbor's connections after adding back-links. Without this, high-degree "hub" nodes accumulate unbounded edges. Graph traversal at those hubs becomes O(hub_degree) instead of O(M), degrading overall search from O(log N) toward O(N).

**The pruning rule:**
- At level 0: max degree = `config.m_max_0` (= 2 * m)
- At level > 0: max degree = `config.m`
- After adding back-link to `neighbor_id` at `level`: count its current edges. If count > limit, fetch all its neighbors, run `select_neighbors` to keep the best `limit`, rewrite its edge list.

**Important:** `set_neighbours` holds `&mut RwTxn`. `select_neighbors` and `get_neighbors` take `&RoTxn`. In heed3, `RwTxn` derefs to `RoTxn` — use `&**txn` or check if `.as_ref()` is available.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_set_neighbours_respects_m_max_0_degree_limit() {
    // Insert M+1 vectors all close to a central hub vector.
    // After all insertions, the hub must have at most m_max_0 edges, not m_max_0+1.
    let (storage, _dir) = test_utils::create_test_storage();
    let arena = bumpalo::Bump::new();
    let mut txn = storage.write_txn().unwrap();
    let config = &storage.vectors.config;
    let m_max_0 = config.m_max_0;

    // Insert hub vector
    let hub_data: Vec<f64> = (0..3).map(|_| 0.0).collect();
    let hub = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &hub_data, None, &arena)
        .unwrap();

    // Insert m_max_0 + 5 satellite vectors all at distance ~0.01 from hub
    for i in 0..(m_max_0 + 5) {
        let mut data = vec![0.0f64; 3];
        data[0] = 0.01 * (i as f64 + 1.0);
        storage.vectors
            .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &data, None, &arena)
            .unwrap();
    }

    // Count hub's edges at level 0 directly from edges_db
    let edge_prefix = sparrow_db::sparrow_engine::vector_core::lmdb::vector_core::VectorCore::out_edges_key(
        hub.id, 0, None
    );
    let hub_degree = storage.vectors.edges_db
        .prefix_iter(&txn, edge_prefix.as_ref()).unwrap()
        .count();

    assert!(
        hub_degree <= m_max_0,
        "hub has {hub_degree} edges at level 0 but m_max_0 = {m_max_0}"
    );

    txn.commit().unwrap();
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test test_set_neighbours_respects_m_max_0_degree_limit 2>&1 | tail -15
```

Expected: FAIL (hub_degree > m_max_0).

- [ ] **Step 3: Add degree pruning to `set_neighbours`**

In `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs`, modify `set_neighbours` (line ~216). After adding the back-link key, add a degree check and pruning step for the neighbor:

```rust
fn set_neighbours<'db: 'arena, 'arena: 'txn, 'txn, 's>(
    &'db self,
    txn: &'txn mut RwTxn<'db>,
    id: u128,
    neighbors: &BinaryHeap<'arena, HVector<'arena>>,
    level: usize,
) -> Result<(), VectorError> {
    let prefix = Self::out_edges_key(id, level, None);

    let mut keys_to_delete: HashSet<Vec<u8>> = self
        .edges_db
        .prefix_iter(txn, prefix.as_ref())?
        .filter_map(|result| result.ok().map(|(key, _)| key.to_vec()))
        .collect();

    neighbors
        .iter()
        .try_for_each(|neighbor| -> Result<(), VectorError> {
            let neighbor_id = neighbor.id;
            if neighbor_id == id {
                return Ok(());
            }

            let out_key = Self::out_edges_key(id, level, Some(neighbor_id));
            keys_to_delete.remove(&out_key);
            self.edges_db.put(txn, &out_key, &())?;

            let in_key = Self::out_edges_key(neighbor_id, level, Some(id));
            keys_to_delete.remove(&in_key);
            self.edges_db.put(txn, &in_key, &())?;

            // ── NEW: Prune neighbor's connections if it exceeds its degree limit ──
            // Limit: m_max_0 at level 0, m at higher levels (standard HNSW)
            let limit = if level == 0 { self.config.m_max_0 } else { self.config.m };
            self.prune_if_over_degree(txn, neighbor_id, neighbor, level, limit)?;
            // ── END NEW ──

            Ok(())
        })?;

    for key in keys_to_delete {
        self.edges_db.delete(txn, &key)?;
    }

    Ok(())
}

/// Prune `node_id`'s outgoing edges at `level` to at most `limit` connections.
/// Called after adding a back-link to ensure hub nodes don't accumulate unbounded edges.
/// This implements the HNSW Algorithm 1 (Malkov & Yashunin), line 14.
fn prune_if_over_degree(
    &self,
    txn: &mut RwTxn,
    node_id: u128,
    node_vec: &HVector<'_>,   // used as the query point for select_neighbors
    level: usize,
    limit: usize,
) -> Result<(), VectorError> {
    let edge_prefix = Self::out_edges_key(node_id, level, None);

    // Count current edges
    let current_count = self
        .edges_db
        .prefix_iter(txn, edge_prefix.as_ref())?
        .count();

    if current_count <= limit {
        return Ok(()); // Fast path: no pruning needed
    }

    // Collect current neighbor IDs
    let neighbor_ids: Vec<u128> = self
        .edges_db
        .prefix_iter(txn, edge_prefix.as_ref())?
        .filter_map(|r| r.ok())
        .filter_map(|(key, _)| {
            if key.len() == 40 {
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&key[24..40]);
                Some(u128::from_be_bytes(arr))
            } else {
                None
            }
        })
        .collect();

    // Build a BinaryHeap of current neighbor vectors for select_neighbors.
    // We use a short-lived arena scoped to this pruning call.
    let prune_arena = bumpalo::Bump::new();
    let mut cands: BinaryHeap<'_, HVector<'_>> = BinaryHeap::with_capacity(&prune_arena, neighbor_ids.len());
    for nid in &neighbor_ids {
        // heed3 RwTxn derefs to RoTxn for reads — use &*txn
        if let Ok(mut v) = self.get_raw_vector_data(&*txn, *nid, node_vec.label, &prune_arena) {
            v.set_distance(v.distance_to(node_vec)?);
            cands.push(v);
        }
    }

    // select_neighbors returns the best `m` (limit) neighbors
    let pruned: BinaryHeap<'_, HVector<'_>> = self.select_neighbors::<fn(&_, &_) -> bool>(
        &*txn,
        node_vec.label,
        node_vec,
        cands,
        level,
        false, // should_extend = false (already have all candidates)
        None,
        &prune_arena,
    )?;

    let keep_ids: HashSet<u128> = pruned.iter().map(|v| v.id).collect();

    // Remove edges not in the keep set (both forward and reverse)
    for nid in &neighbor_ids {
        if !keep_ids.contains(nid) {
            let fwd = Self::out_edges_key(node_id, level, Some(*nid));
            let rev = Self::out_edges_key(*nid, level, Some(node_id));
            let _ = self.edges_db.delete(txn, &fwd);
            let _ = self.edges_db.delete(txn, &rev);
        }
    }

    Ok(())
}
```

> **Note:** `select_neighbors`'s return type is `BinaryHeap<'arena, HVector<'arena>>`. The arena here has a shorter lifetime than `'db`. Adjust lifetimes as the compiler requires — the key constraint is that `prune_arena` outlives the `pruned` value, which it does since both are in the same scope.

- [ ] **Step 4: Run the test**

```bash
cargo test test_set_neighbours_respects_m_max_0_degree_limit 2>&1 | tail -15
cargo check --workspace 2>&1 | tail -5
```

Expected: test passes, no compile errors.

- [ ] **Step 5: Run full vector test suite**

```bash
cargo test --test '*' -p sparrow-db 2>&1 | tail -20
```

Expected: no regressions in existing vector tests.

- [ ] **Step 6: Commit**

```bash
git add sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs
git commit -m "fix(hnsw): enforce m_max_0 degree limit on back-links in set_neighbours — prevents hub-node degradation"
```

---

## Task 3: Fix O(N²) MCP Pagination

**Files:**
- Modify: `sparrow-db/src/sparrow_gateway/mcp/mcp.rs`

**Background:** The `MCPConnection` struct stores a `query_chain` and `current_position`. Each call to `next()` (line 338) re-executes the full `execute_query_chain` traversal from scratch and calls `stream.nth(current_position)` to skip to the current page position. For the Nth item, this performs N full traversals — O(N²) total work.

**Fix:** Materialize the full result set on the first `next()` call. Store serialized JSON results in `MCPConnection`. Subsequent calls index directly into the stored Vec.

**Why JSON bytes and not owned `TraversalValue`:** `TraversalValue<'arena>` contains arena-allocated string slices that cannot escape the request scope. Serializing to `Vec<u8>` (JSON) on first call stores results as owned bytes that survive across requests with zero lifetime concerns.

- [ ] **Step 1: Write the failing benchmark/test**

```rust
#[test]
fn test_mcp_next_does_not_reexecute_query_chain() {
    // Insert 20 nodes. Call next() 10 times.
    // With the fix: execute_query_chain is called once.
    // We can verify indirectly: the 10th call must complete in < 10x the time of the 1st call.
    // Alternatively, use a call counter via a mock or just assert result correctness.

    // Correctness test: paging through 20 nodes via next() must return each node exactly once.
    // (Implementation detail: if query is re-executed from scratch, results may differ due to
    //  MVCC snapshot differences — but in practice they're stable. Test correctness, not timing.)

    // Setup: create connection, insert 20 nodes, run query_chain
    // Call next() 20 times, collect all returned IDs
    // Assert: 20 distinct IDs returned, no duplicates, no missing
    todo!("implement using existing MCP test infrastructure");
}
```

> Find existing MCP handler tests in `sparrow-db/src/sparrow_gateway/tests/` and replicate the setup pattern.

- [ ] **Step 2: Add `cached_results` to `MCPConnection`**

In `sparrow-db/src/sparrow_gateway/mcp/mcp.rs`, find `struct MCPConnection` (around line 90):

```rust
// Before:
pub struct MCPConnection {
    pub query_chain: Vec<QueryStep>,
    pub current_position: usize,
}

// After:
pub struct MCPConnection {
    pub query_chain: Vec<QueryStep>,
    pub current_position: usize,
    /// Materialized result cache. None = not yet executed.
    /// Populated on first next() call, then indexed directly.
    pub cached_results: Option<Vec<Vec<u8>>>, // each element is a JSON-serialized result
}

impl MCPConnection {
    pub fn new() -> Self {
        Self {
            query_chain: Vec::new(),
            current_position: 0,
            cached_results: None,
        }
    }

    pub fn reset(&mut self) {
        self.current_position = 0;
        self.cached_results = None; // Force re-execution on next() after reset
    }

    pub fn clear(&mut self) {
        self.query_chain.clear();
        self.current_position = 0;
        self.cached_results = None;
    }
}
```

Update the existing `new()`, `reset()`, and `clear()` methods to initialize/clear `cached_results`. Check what methods exist on MCPConnection and update them all.

- [ ] **Step 3: Rewrite the `next()` handler to use the cache**

Find the `next()` handler function (around line 305). Replace the core logic:

```rust
// Current (O(N²)):
let stream = execute_query_chain(&query_chain, storage, &txn, &arena)?;
let next_value = match stream.nth(current_position)? { ... }

// Replace with:
// Lock connections just long enough to get/set cache
let cached_bytes = {
    let connections = input.mcp_connections.lock().unwrap();
    let conn = connections.get_connection(&data.connection_id)
        .ok_or_else(|| GraphError::StorageError("Connection not found".into()))?;
    conn.cached_results.clone() // None if not yet materialized
};

let result_bytes = if let Some(ref cache) = cached_bytes {
    // Fast path: return from cache
    cache.get(current_position).cloned()
} else {
    // First call: execute once, collect all results, serialize to JSON bytes, cache them
    let arena = Bump::new();
    let storage = input.mcp_backend.db.as_ref();
    let txn = storage.graph_env.read_txn()?;
    let stream = execute_query_chain(&query_chain, storage, &txn, &arena)?;

    let all_results: Vec<Vec<u8>> = stream
        .map(|item| -> Result<Vec<u8>, GraphError> {
            let item = item?;
            // Serialize each item to JSON bytes for owned storage
            sonic_rs::to_vec(&item)
                .map_err(|e| GraphError::Other(format!("serialization error: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let item = all_results.get(current_position).cloned();

    // Store in connection cache
    {
        let mut connections = input.mcp_connections.lock().unwrap();
        if let Some(conn) = connections.get_connection_mut(&data.connection_id) {
            conn.cached_results = Some(all_results);
        }
    }

    item
};

// Update position and return result
match result_bytes {
    Some(bytes) => {
        {
            let mut connections = input.mcp_connections.lock().unwrap();
            if let Some(conn) = connections.get_connection_mut(&data.connection_id) {
                conn.current_position += 1;
            }
        }
        // Deserialize and return...
        let value: sonic_rs::Value = sonic_rs::from_slice(&bytes)?;
        // build and return the MCP response with value
    }
    None => {
        // End of results
        // ...return end-of-stream response
    }
}
```

> **Note:** Check how the current `next()` handler builds its return value and replicate that construction. The key change is: `execute_query_chain` is called at most once per connection per query chain. Adapt the exact serialization format to match what the MCP client expects — look at how the existing code serializes `TraversalValue` and replicate it when building `all_results`.

> **Memory note:** For large result sets, the cache can be large. This is acceptable for MCP use cases (AI agent pagination typically deals with hundreds of results, not millions). If needed, add a configurable cap (e.g., 10,000 results max) and return an error if exceeded.

- [ ] **Step 4: Ensure `reset()` clears the cache**

When the query chain is updated (new search), `cached_results` must be cleared so the next `next()` call re-executes. Verify all places that call `connection.reset()` or `connection.query_chain.clear()` also set `cached_results = None`. The updated struct methods from Step 2 handle this — confirm the call sites use those methods.

- [ ] **Step 5: Verify cargo check and run tests**

```bash
cargo check --workspace 2>&1 | tail -5
cargo test --lib -p sparrow-db -- mcp 2>&1 | tail -20
```

- [ ] **Step 6: Commit**

```bash
git add sparrow-db/src/sparrow_gateway/mcp/mcp.rs
git commit -m "fix(mcp): materialize query results on first next() call — eliminates O(N²) pagination re-execution"
```

---

## Task 4: Fix Embedding `block_on` Worker Thread Deadlock

**Files:**
- Modify: `sparrow-db/src/sparrow_gateway/embedding_providers/mod.rs`
- Modify: `sparrow-db/src/sparrow_gateway/mcp/mcp.rs` (call sites)

**Background:** `EmbeddingModelImpl::fetch_embedding` (line 174) calls `handle.block_on(self.fetch_embedding_async(text))`. This blocks the OS thread for the full duration of an HTTP request to an external embedding provider (OpenAI, etc.). The worker pool uses OS threads (not tokio tasks). If all workers block simultaneously on external HTTP, no new queries can be processed — the database freezes.

**Root cause:** The sync worker path calls `fetch_embedding` (sync), which uses `block_on` to drive an async HTTP call from a thread context. This is the anti-pattern.

**Fix:** Remove the sync `fetch_embedding` method. All embedding calls must go through the async path. At call sites in the MCP handlers (which already run in async context), call `fetch_embedding_async` directly. At call sites in sync handlers, refactor to pre-compute the embedding in the async axum handler before dispatching to the worker.

- [ ] **Step 1: Write the test**

```rust
// This is a compile-level test: verify that fetch_embedding (sync) no longer exists.
// If the method is removed, any call site still using it will fail to compile, surfacing
// all remaining usages that need to be migrated.
// No runtime test needed — the bug is structural.
```

Instead of a runtime test, the fix is verified by: (a) removing the sync method, (b) cargo check failing at all remaining call sites, (c) fixing each call site, (d) cargo check passing.

- [ ] **Step 2: Find all `fetch_embedding` call sites**

```bash
grep -rn "\.fetch_embedding\b" /Users/franciscobaptista/Development/SparrowDB/sparrow-db/src/ --include="*.rs"
```

List each call site and determine: is it in an `async fn`? If yes → change to `fetch_embedding_async(...).await`. If in a sync fn → refactor to push the embedding call up to the nearest async boundary.

- [ ] **Step 3: Remove the blocking `fetch_embedding` method**

In `sparrow-db/src/sparrow_gateway/embedding_providers/mod.rs`:

Remove from the `EmbeddingModel` trait:
```rust
// DELETE THIS:
fn fetch_embedding(&self, text: &str) -> Result<Vec<f64>, GraphError>;
```

Remove the implementation in `EmbeddingModelImpl`:
```rust
// DELETE THIS:
fn fetch_embedding(&self, text: &str) -> Result<Vec<f64>, GraphError> {
    let handle = tokio::runtime::Handle::current();
    handle.block_on(self.fetch_embedding_async(text))
}
```

Remove the macros at lines ~460–468 that call `embedding_model.fetch_embedding(...)` and replace them with async-aware equivalents (see Step 5).

- [ ] **Step 4: Fix MCP call sites**

In `sparrow-db/src/sparrow_gateway/mcp/mcp.rs`, find line 953:
```rust
.fetch_embedding(&req.data.query)?
```

This is inside an `async fn` (MCP handlers are async). Replace with:
```rust
.fetch_embedding_async(&req.data.query).await
    .map_err(|e| GraphError::EmbeddingError(e.to_string()))?
```

Search for all other `.fetch_embedding(` calls in the file and apply the same change.

- [ ] **Step 5: Fix macro call sites**

In `embedding_providers/mod.rs`, find the macros (lines ~460–468) that call `embedding_model.fetch_embedding($query)?`. These macros are likely used in both async and sync contexts. 

**Option A (preferred):** If the macros are only used in async contexts, change them to await:
```rust
macro_rules! get_embedding {
    ($model:expr, $query:expr) => {
        $model.fetch_embedding_async($query).await?
    };
}
```

**Option B:** If some callers are in sync contexts, refactor those callers to async, or pass a pre-computed `Vec<f64>` into the sync handler instead of fetching inside it.

To determine which: `grep -rn "get_embedding!\|fetch_embedding" sparrow-db/src/ --include="*.rs"` and check each call site.

- [ ] **Step 6: Run cargo check — fix all compile errors**

```bash
cargo check --workspace 2>&1 | grep "error" | head -20
```

Fix each remaining `fetch_embedding` reference until the workspace compiles clean.

- [ ] **Step 7: Run embedding tests**

```bash
cargo test embedding 2>&1 | tail -15
```

Expected: all existing embedding tests pass.

- [ ] **Step 8: Commit**

```bash
git add sparrow-db/src/sparrow_gateway/embedding_providers/mod.rs \
        sparrow-db/src/sparrow_gateway/mcp/mcp.rs
git commit -m "fix(embedding): remove blocking fetch_embedding — all embedding calls now go through async path, preventing worker thread deadlock"
```

---

## Self-Review

**Spec coverage:**

| Bug | Task | Fix |
|---|---|---|
| BM25 NaN when empty doc + no prior docs | 1 | Guard `avgdl=0, doc_len=0` → use `1.0` ratio |
| HNSW hub-node unbounded degree | 2 | `prune_if_over_degree` after back-link insertion |
| O(N²) MCP pagination | 3 | Materialize results on first `next()`, cache in MCPConnection |
| Worker thread `block_on` deadlock | 4 | Remove sync `fetch_embedding`, use `fetch_embedding_async` everywhere |

**What was intentionally left out:**
- `Loc.span: String` memory amplification — deleted in Chumsky rewrite (Project 2)
- `Value` arithmetic overflow — in Project 1 Task 3
- Dual-engine (AOT vs interpreter) parity — architectural, needs separate investigation
- RocksDB secondary index — in Project 1 Task 4

---

## Project Roadmap

| Project | Contents | Status |
|---|---|---|
| **Project 0** (this plan) | BM25 NaN, HNSW degree, O(N²) pagination, block_on deadlock | Ready to execute |
| **Project 1** | Vector hard_delete, Value overflow, entry point drift, diagnostics, endpoints, rebuild | Plan written |
| **Project 2** | Chumsky parser (replaces Pest): Pratt precedence, error recovery, string interning, generic attributes, CST/AST separation | Next quarter |
