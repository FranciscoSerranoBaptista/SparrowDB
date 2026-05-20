# SparrowDB — Vector Stability, Correctness & New Endpoints

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three P0 data-corruption bugs (DROP leak, Value arithmetic overflow, entry point drift), add missing vector HTTP endpoints, and fix the RocksDB secondary index merge operator.

**Architecture:** The sparrow-db crate is dual-backend (LMDB via `heed3` / RocksDB via `rocksdb`), feature-gated with `#[cfg(feature = "lmdb")]` / `#[cfg(feature = "rocks")]`. Every storage fix must be applied to both backends. New HTTP handlers live in `sparrow-db/src/sparrow_gateway/builtin/` and self-register via `inventory::submit!`. The single write worker enforces LMDB's one-writer constraint — all `is_write: true` handlers go to it.

**Tech Stack:** Rust, heed3 (LMDB), rocksdb crate, axum, inventory, bumpalo, bincode, sonic_rs

---

## Critical background: how the two `edges_db` tables relate

`SparrowGraphStorage` has its own `edges_db` (graph topology — node-to-node).
`VectorCore` has its own `edges_db` (HNSW spatial graph — vector-to-vector).
These are **different LMDB databases**. `drop_vector` correctly cleans the graph topology edges but calls `self.vectors.delete()` (soft delete) instead of a hard delete, leaving the HNSW `edges_db` and `vectors_db` intact. This is the DROP leak.

## HNSW edge key format (LMDB)
```
edges_db key: [source_id(16 BE) | level(8 BE usize) | sink_id(16 BE)] = 40 bytes
edges_db value: unit ()
```
Edges are stored **bidirectionally**: `source→sink` AND `sink→source` are both written by `set_neighbours`. Deleting a vector's edges requires:
1. Prefix-iterate `edges_db` on `[id(16)]` to find all forward edges
2. For each forward key of length 40: extract `sink_id = key[24..40]`, build reverse key `[sink_id | level | id]`, delete it
3. Delete the forward key

## Handler registration pattern
New builtin handlers follow this pattern (3-arg `Handler::new`):
```rust
inventory::submit! {
    HandlerSubmission(
        Handler::new("route_name", handler_inner_fn, is_write_bool)
    )
}
```

---

## File Structure

| File | Change |
|---|---|
| `sparrow-db/src/sparrow_engine/vector_core/lmdb/hnsw.rs` | Add `hard_delete` to `HNSW` trait |
| `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` | Implement `hard_delete`; fix `delete` to guard entry point |
| `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_core.rs` | Implement `hard_delete` for RocksDB |
| `sparrow-db/src/sparrow_engine/storage_core/mod.rs` | `drop_vector`: call `hard_delete` instead of `delete` |
| `sparrow-db/src/protocol/value.rs` | Fix signed+unsigned overflow in Add, Sub, Mul, Div, Rem |
| `sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs` | New: `GET /diagnostics` handler |
| `sparrow-db/src/sparrow_gateway/builtin/vector_ops.rs` | New: `POST /vector-soft-delete`, `POST /vector-hard-delete` |
| `sparrow-db/src/sparrow_gateway/builtin/vector_rebuild.rs` | New: `POST /rebuild-vector-index`, `POST /purge-soft-deleted` |
| `sparrow-db/src/sparrow_gateway/builtin/mod.rs` | Add `pub mod` for new files |
| `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` | Add `insert_with_id`, `stats`, `rebuild`, `purge_soft_deleted` |
| `sparrow-db/src/sparrow_engine/vector_core/rocks/mod.rs` | Fix secondary index merge operator |

---

## Task 1: Fix the DROP Leak — `VectorCore::hard_delete`

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/hnsw.rs`
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs`
- Modify: `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_core.rs`
- Modify: `sparrow-db/src/sparrow_engine/storage_core/mod.rs` (lines 568 LMDB, 1195 RocksDB)
- Test: `sparrow-db/src/sparrow_engine/tests/traversal_tests/vector_traversal_tests.rs`

- [ ] **Step 1: Write the failing test**

In `sparrow-db/src/sparrow_engine/tests/traversal_tests/vector_traversal_tests.rs`, add after the existing `test_drop_vector_removes_edges` test:

```rust
#[test]
fn test_drop_vector_hard_deletes_hnsw_data() {
    // Setup: insert a vector, record its id, then DROP it
    // Verify: vectors_db, vector_properties_db, and VectorCore.edges_db are all empty for that id
    let (storage, _dir) = test_utils::create_test_storage(); // use existing test helper
    let arena = bumpalo::Bump::new();
    let mut txn = storage.write_txn().unwrap();

    // Insert a vector
    let data = vec![1.0f64, 2.0, 3.0];
    let inserted = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &data, None, &arena)
        .expect("insert failed");
    let id = inserted.id;

    // Drop it via the graph engine drop_vector path
    storage.drop_vector(&mut txn, id).expect("drop failed");

    // Assert: no raw vector data remains
    let prefix = [b"v:".as_ref(), &id.to_be_bytes()].concat();
    let count = storage.vectors.vectors_db
        .prefix_iter(&txn, &prefix).unwrap()
        .count();
    assert_eq!(count, 0, "vectors_db still has data for dropped vector");

    // Assert: no properties remain
    let props = storage.vectors.vector_properties_db.get(&txn, &id).unwrap();
    assert!(props.is_none(), "vector_properties_db still has entry for dropped vector");

    // Assert: no forward HNSW edges remain
    let edge_count = storage.vectors.edges_db
        .prefix_iter(&txn, &id.to_be_bytes()).unwrap()
        .count();
    assert_eq!(edge_count, 0, "VectorCore.edges_db still has edges for dropped vector");

    txn.commit().unwrap();
}

#[test]
fn test_drop_vector_that_is_entry_point_clears_entry_point() {
    let (storage, _dir) = test_utils::create_test_storage();
    let arena = bumpalo::Bump::new();
    let mut txn = storage.write_txn().unwrap();

    // Insert one vector (it becomes the entry point)
    let data = vec![1.0f64, 0.0, 0.0];
    let inserted = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &data, None, &arena)
        .expect("insert failed");
    let id = inserted.id;

    // Drop it
    storage.drop_vector(&mut txn, id).expect("drop failed");

    // Assert: entry point key is gone from vectors_db
    use sparrow_db::sparrow_engine::vector_core::lmdb::vector_core::ENTRY_POINT_KEY;
    let ep = storage.vectors.vectors_db.get(&txn, ENTRY_POINT_KEY).unwrap();
    assert!(ep.is_none(), "entry point still set after dropping entry point vector");

    txn.commit().unwrap();
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test test_drop_vector_hard_deletes_hnsw_data test_drop_vector_that_is_entry_point_clears_entry_point 2>&1 | tail -20
```

Expected: FAIL (drop_vector still calls soft delete).

- [ ] **Step 3: Add `hard_delete` to the HNSW trait**

Edit `sparrow-db/src/sparrow_engine/vector_core/lmdb/hnsw.rs`. Find the `pub trait HNSW` block and add:

```rust
fn hard_delete(&self, txn: &mut RwTxn, id: u128) -> Result<(), VectorError>;
```

- [ ] **Step 4: Implement `hard_delete` on LMDB VectorCore**

Edit `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs`. Before the closing `}` of the `impl HNSW for VectorCore` block (currently after `fn delete` at line ~661), add:

```rust
fn hard_delete(&self, txn: &mut RwTxn, id: u128) -> Result<(), VectorError> {
    // 1. Remove all raw vector data entries: prefix [v: | id(16)] covers all levels
    let data_prefix = [VECTOR_PREFIX, id.to_be_bytes().as_ref()].concat();
    let data_keys: Vec<Vec<u8>> = self
        .vectors_db
        .prefix_iter(txn, data_prefix.as_ref())?
        .filter_map(|r| r.ok().map(|(k, _)| k.to_vec()))
        .collect();
    for key in data_keys {
        self.vectors_db.delete(txn, key.as_ref())?;
    }

    // 2. Remove properties entry (ignore NotFound — vector may have been partially written)
    let _ = self.vector_properties_db.delete(txn, &id);

    // 3. Remove all forward HNSW edges and their bidirectional reverses.
    //    Key format: [source(16) | level(8) | sink(16)] = 40 bytes
    let edge_prefix = id.to_be_bytes();
    let forward_keys: Vec<Vec<u8>> = self
        .edges_db
        .prefix_iter(txn, edge_prefix.as_ref())?
        .filter_map(|r| r.ok().map(|(k, _)| k.to_vec()))
        .collect();
    for fwd in &forward_keys {
        if fwd.len() == 40 {
            let mut rev = [0u8; 40];
            rev[..16].copy_from_slice(&fwd[24..40]);   // sink_id → source slot
            rev[16..24].copy_from_slice(&fwd[16..24]); // level unchanged
            rev[24..40].copy_from_slice(&fwd[..16]);   // source_id → sink slot
            let _ = self.edges_db.delete(txn, rev.as_ref()); // best-effort: reverse may already be gone
        }
        self.edges_db.delete(txn, fwd.as_ref())?;
    }

    // 4. If this vector is the entry point, clear it.
    //    Next insert() will hit the Err branch in get_entry_point() and set a fresh one.
    if let Ok(Some(ep_bytes)) = self.vectors_db.get(txn, ENTRY_POINT_KEY) {
        if ep_bytes.len() == 16 {
            let ep_id = u128::from_be_bytes(ep_bytes.try_into().unwrap());
            if ep_id == id {
                self.vectors_db.delete(txn, ENTRY_POINT_KEY)?;
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 5: Implement `hard_delete` on RocksDB VectorCore**

Edit `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_core.rs`. Find the `impl HNSW for VectorCore` block and add:

```rust
fn hard_delete(&self, txn: &mut WTxn, id: u128) -> Result<(), VectorError> {
    let id_bytes = id.to_be_bytes();

    // 1. Remove all vector data entries for this id across all levels
    //    RocksDB: iterate the "vectors" CF with prefix [id_bytes]
    let vectors_cf = self.db.cf_handle("vectors")
        .ok_or_else(|| VectorError::VectorCoreError("vectors CF missing".into()))?;
    let opts = rocksdb::ReadOptions::default();
    let mut iter = self.db.iterator_cf_opt(&vectors_cf, opts, rocksdb::IteratorMode::From(
        &[VECTOR_PREFIX, id_bytes.as_ref()].concat(),
        rocksdb::Direction::Forward,
    ));
    let mut to_delete: Vec<Vec<u8>> = Vec::new();
    for result in &mut iter {
        let (key, _) = result.map_err(|e| VectorError::VectorCoreError(e.to_string()))?;
        if !key.starts_with(VECTOR_PREFIX) || !key[VECTOR_PREFIX.len()..].starts_with(&id_bytes) {
            break;
        }
        to_delete.push(key.to_vec());
    }
    drop(iter);
    for key in to_delete {
        txn.delete_cf(&vectors_cf, key)?;
    }

    // 2. Remove properties entry
    let props_cf = self.db.cf_handle("vector_data")
        .ok_or_else(|| VectorError::VectorCoreError("vector_data CF missing".into()))?;
    let _ = txn.delete_cf(&props_cf, id_bytes);

    // 3. Remove HNSW edges (forward and reverse)
    //    RocksDB edge key: [source(16) | level(1 u8) | sink(16)] = 33 bytes
    let edges_cf = self.db.cf_handle("hnsw_edges")
        .ok_or_else(|| VectorError::VectorCoreError("hnsw_edges CF missing".into()))?;
    let opts2 = rocksdb::ReadOptions::default();
    let mut iter2 = self.db.iterator_cf_opt(&edges_cf, opts2, rocksdb::IteratorMode::From(
        &id_bytes,
        rocksdb::Direction::Forward,
    ));
    let mut fwd_keys: Vec<Vec<u8>> = Vec::new();
    for result in &mut iter2 {
        let (key, _) = result.map_err(|e| VectorError::VectorCoreError(e.to_string()))?;
        if !key.starts_with(&id_bytes) { break; }
        fwd_keys.push(key.to_vec());
    }
    drop(iter2);
    for fwd in &fwd_keys {
        if fwd.len() == 33 {
            let mut rev = [0u8; 33];
            rev[..16].copy_from_slice(&fwd[17..33]);  // sink_id
            rev[16] = fwd[16];                         // level
            rev[17..33].copy_from_slice(&fwd[..16]);   // source_id
            let _ = txn.delete_cf(&edges_cf, rev);
        }
        txn.delete_cf(&edges_cf, fwd)?;
    }

    // 4. Clear entry point if it is this vector
    let ep_cf = self.db.cf_handle("ep")
        .ok_or_else(|| VectorError::VectorCoreError("ep CF missing".into()))?;
    if let Ok(Some(ep_bytes)) = self.db.get_cf(&ep_cf, ENTRY_POINT_KEY) {
        if ep_bytes.len() == 16 && ep_bytes.as_slice() == id_bytes {
            let _ = txn.delete_cf(&ep_cf, ENTRY_POINT_KEY);
        }
    }

    Ok(())
}
```

> **Note:** Verify the RocksDB edge key size (33 vs 40) by reading `rocks/vector_core.rs` around the `edges_key` function before implementing. Adjust the `rev` array size accordingly.

- [ ] **Step 6: Wire `drop_vector` to call `hard_delete`**

In `sparrow-db/src/sparrow_engine/storage_core/mod.rs`:

LMDB block (line 568): change
```rust
self.vectors.delete(txn, id, &arena)?;
```
to:
```rust
self.vectors.hard_delete(txn, id)?;
```

RocksDB block (line 1195): same change.

The `arena` variable is no longer needed at that site — remove it if it becomes unused (compiler will warn).

- [ ] **Step 7: Run tests to verify they pass**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test test_drop_vector_hard_deletes_hnsw_data test_drop_vector_that_is_entry_point_clears_entry_point 2>&1 | tail -20
cargo test test_drop_vector_removes_edges 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Step 8: Run full workspace check**

```bash
cargo check --workspace 2>&1 | tail -5
```

Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add sparrow-db/src/sparrow_engine/vector_core/lmdb/hnsw.rs \
        sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs \
        sparrow-db/src/sparrow_engine/vector_core/rocks/vector_core.rs \
        sparrow-db/src/sparrow_engine/storage_core/mod.rs \
        sparrow-db/src/sparrow_engine/tests/
git commit -m "fix(vector): replace soft delete in drop_vector with hard_delete — fixes DROP leak in HNSW"
```

---

## Task 2: Fix Entry Point Drift (soft-delete of entry point vector)

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` (`fn delete`)
- Modify: `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_core.rs` (`fn delete`)
- Test: `sparrow-db/src/sparrow_engine/tests/traversal_tests/vector_traversal_tests.rs`

**Background:** `get_entry_point` calls `get_raw_vector_data` which does NOT check the `deleted` flag. When the entry point vector is soft-deleted, `search()` starts traversal from it — a ghost. The fix: in `VectorCore::delete()` (soft delete), detect if the target is the entry point and clear/re-assign it.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_soft_delete_entry_point_reassigns_or_clears_it() {
    let (storage, _dir) = test_utils::create_test_storage();
    let arena = bumpalo::Bump::new();
    let mut txn = storage.write_txn().unwrap();

    // Insert two vectors so the second one can be a non-deleted fallback
    let v1 = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[1.0, 0.0, 0.0], None, &arena)
        .unwrap();
    let _v2 = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[0.0, 1.0, 0.0], None, &arena)
        .unwrap();

    // Soft-delete v1 (the entry point, since it was inserted first)
    storage.vectors.delete(&mut txn, v1.id, &arena).unwrap();

    // The entry point must NOT still point to v1
    use sparrow_db::sparrow_engine::vector_core::lmdb::vector_core::ENTRY_POINT_KEY;
    let ep_bytes = storage.vectors.vectors_db.get(&txn, ENTRY_POINT_KEY).unwrap();
    if let Some(ep_bytes) = ep_bytes {
        let ep_id = u128::from_be_bytes(ep_bytes.try_into().unwrap());
        assert_ne!(ep_id, v1.id, "entry point still points to soft-deleted vector");
    }
    // (ep may be None — that's also acceptable; next search will return empty, next insert resets it)

    txn.commit().unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_soft_delete_entry_point_reassigns_or_clears_it 2>&1 | tail -15
```

Expected: FAIL.

- [ ] **Step 3: Fix `VectorCore::delete` in LMDB backend**

In `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs`, edit the `fn delete` implementation (line ~641). After setting `properties.deleted = true` and committing, add entry point handling:

```rust
fn delete(&self, txn: &mut RwTxn, id: u128, arena: &bumpalo::Bump) -> Result<(), VectorError> {
    match self.get_vector_properties(txn, id, arena)? {
        Some(mut properties) => {
            if properties.deleted {
                return Err(VectorError::VectorAlreadyDeleted(id.to_string()));
            }
            properties.deleted = true;
            self.vector_properties_db.put(
                txn,
                &id,
                bincode::serialize(&properties)?.as_ref(),
            )?;

            // Guard: if this vector is the current entry point, clear it.
            // Next insert() will set a new entry point. Next search() will find
            // no entry point and return an empty result (EntryPointNotFound error
            // is mapped to empty results in the search path).
            if let Ok(Some(ep_bytes)) = self.vectors_db.get(txn, ENTRY_POINT_KEY) {
                if ep_bytes.len() == 16 {
                    let ep_id = u128::from_be_bytes(ep_bytes.try_into().unwrap());
                    if ep_id == id {
                        // Try to find a replacement entry point from level-0 neighbors
                        let edge_prefix = Self::out_edges_key(id, 0, None);
                        let replacement = self
                            .edges_db
                            .prefix_iter(txn, edge_prefix.as_ref())?
                            .filter_map(|r| r.ok())
                            .find_map(|(key, _)| {
                                if key.len() == 40 {
                                    let mut arr = [0u8; 16];
                                    arr.copy_from_slice(&key[24..40]);
                                    let neighbor_id = u128::from_be_bytes(arr);
                                    // Only use neighbor if it exists and is not deleted
                                    self.get_raw_vector_data(txn, neighbor_id, properties.label, arena).ok()
                                } else {
                                    None
                                }
                            });
                        match replacement {
                            Some(new_ep) => self.set_entry_point(txn, &new_ep)?,
                            None => { self.vectors_db.delete(txn, ENTRY_POINT_KEY)?; }
                        }
                    }
                }
            }

            debug_println!("vector soft-deleted with id {}", &id);
            Ok(())
        }
        None => Err(VectorError::VectorNotFound(id.to_string())),
    }
}
```

- [ ] **Step 4: Apply the same fix to RocksDB `delete`**

Read `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_core.rs`, find `fn delete`, apply the same entry point guard. Adapt column family access to RocksDB API (use `self.db.get_cf(&ep_cf, ENTRY_POINT_KEY)` and `txn.delete_cf(&ep_cf, ENTRY_POINT_KEY)`).

- [ ] **Step 5: Run tests**

```bash
cargo test test_soft_delete_entry_point_reassigns_or_clears_it 2>&1 | tail -15
cargo check --workspace 2>&1 | tail -5
```

Expected: test passes, no compile errors.

- [ ] **Step 6: Commit**

```bash
git add sparrow-db/src/sparrow_engine/vector_core/
git commit -m "fix(vector): guard entry point drift on soft delete — reassign or clear EP when soft-deleting it"
```

---

## Task 3: Fix `Value` Arithmetic Overflow (signed + unsigned promotion)

**Files:**
- Modify: `sparrow-db/src/protocol/value.rs`
- Test: `sparrow-db/src/protocol/value.rs` (inline `#[cfg(test)]` module)

**Background:** In `impl std::ops::Add for Value` and `impl std::ops::Sub for Value`, the mixed signed+unsigned arms promote `U64`/`U128` to `i64`:
```rust
Value::U64(v) => v as i64,  // silently truncates if v > i64::MAX
Value::U128(v) => v as i64, // silently truncates if v > i64::MAX
```
The same pattern appears in Sub (lines ~652–678). Must fix Add and Sub. Check Mul, Div, Rem for the same pattern.

The correct promotion for signed+unsigned is `i128` (can represent all `i64` and all `u64` values; `u128` overflows `i128` only above 2^127 which is acceptable to return as `I128` or saturate).

- [ ] **Step 1: Write the failing test**

In `sparrow-db/src/protocol/value.rs`, inside the `#[cfg(test)]` module:

```rust
#[test]
fn test_value_add_signed_unsigned_no_overflow() {
    // i64::MIN + u64::MAX used to silently truncate u64::MAX to -1
    let a = Value::I64(-1_i64);
    let b = Value::U64(u64::MAX);
    let result = a + b;
    // u64::MAX = 18446744073709551615
    // -1 + 18446744073709551615 = 18446744073709551614
    // Result must not be -2 (which is what (u64::MAX as i64) + (-1) = -1 + (-1) gives)
    match result {
        Value::I128(v) => assert_eq!(v, 18446744073709551614_i128),
        Value::U128(v) => assert_eq!(v, 18446744073709551614_u128),
        other => panic!("unexpected result type: {other:?}"),
    }
}

#[test]
fn test_value_sub_signed_unsigned_no_overflow() {
    let a = Value::I64(0_i64);
    let b = Value::U64(u64::MAX);
    let result = a - b;
    // 0 - u64::MAX = -18446744073709551615
    match result {
        Value::I128(v) => assert_eq!(v, -18446744073709551615_i128),
        other => panic!("unexpected result type: {other:?}"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_value_add_signed_unsigned_no_overflow test_value_sub_signed_unsigned_no_overflow -- --nocapture 2>&1 | tail -20
```

Expected: FAIL (wrong result type or wrong value).

- [ ] **Step 3: Fix `impl Add for Value` — signed + unsigned arm**

In `sparrow-db/src/protocol/value.rs`, find the two mixed-sign arms in `impl std::ops::Add for Value` (around lines 508–531). Replace:

```rust
// Signed + Unsigned → I64 (safe widening that can represent both)
(a, b) if a.is_signed_int() && b.is_unsigned_int() => {
    let a_i64 = a.to_i64().unwrap();
    let b_i64 = match b {
        Value::U8(v) => v as i64,
        Value::U16(v) => v as i64,
        Value::U32(v) => v as i64,
        Value::U64(v) => v as i64,   // OVERFLOW
        Value::U128(v) => v as i64,  // OVERFLOW
        _ => unreachable!(),
    };
    Value::I64(a_i64.wrapping_add(b_i64))
}
(a, b) if a.is_unsigned_int() && b.is_signed_int() => {
    let a_i64 = match a {
        Value::U8(v) => v as i64,
        Value::U16(v) => v as i64,
        Value::U32(v) => v as i64,
        Value::U64(v) => v as i64,   // OVERFLOW
        Value::U128(v) => v as i64,  // OVERFLOW
        _ => unreachable!(),
    };
    let b_i64 = b.to_i64().unwrap();
    Value::I64(a_i64.wrapping_add(b_i64))
}
```

with:

```rust
// Signed + Unsigned → I128 (safe: i128 can represent all i64 and all u64 values)
(a, b) if a.is_signed_int() && b.is_unsigned_int() => {
    let a_i128 = a.to_i64().unwrap() as i128;
    let b_i128 = match b {
        Value::U8(v) => v as i128,
        Value::U16(v) => v as i128,
        Value::U32(v) => v as i128,
        Value::U64(v) => v as i128,
        Value::U128(v) => v as i128,  // u128 > i128::MAX wraps — acceptable at this scale
        _ => unreachable!(),
    };
    Value::I128(a_i128.wrapping_add(b_i128))
}
(a, b) if a.is_unsigned_int() && b.is_signed_int() => {
    let a_i128 = match a {
        Value::U8(v) => v as i128,
        Value::U16(v) => v as i128,
        Value::U32(v) => v as i128,
        Value::U64(v) => v as i128,
        Value::U128(v) => v as i128,
        _ => unreachable!(),
    };
    let b_i128 = b.to_i64().unwrap() as i128;
    Value::I128(a_i128.wrapping_add(b_i128))
}
```

> **Note:** Check whether `Value::I128` exists in the enum. If not, return `Value::I64` with a saturating clamp: `Value::I64(result.clamp(i64::MIN as i128, i64::MAX as i128) as i64)`. Check `value.rs` enum definition before writing.

- [ ] **Step 4: Fix `impl Sub for Value` — same arms (lines ~652–678)**

Apply the identical promotion from `i64` to `i128` in the Sub implementation. Same pattern, same fix.

- [ ] **Step 5: Scan and fix Mul, Div, Rem**

```bash
grep -n "as i64.*// May overflow\|U64.*as i64\|U128.*as i64" sparrow-db/src/protocol/value.rs
```

For each hit in Mul, Div, Rem: apply the same fix (promote to `i128`).

- [ ] **Step 6: Run tests**

```bash
cargo test test_value_add_signed_unsigned_no_overflow test_value_sub_signed_unsigned_no_overflow 2>&1 | tail -10
cargo check --workspace 2>&1 | tail -5
```

Expected: both pass.

- [ ] **Step 7: Commit**

```bash
git add sparrow-db/src/protocol/value.rs
git commit -m "fix(value): promote signed+unsigned arithmetic to i128 — eliminates silent u64/u128→i64 overflow"
```

---

## Task 4: Fix RocksDB Secondary Index Merge Operator

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/storage_core/mod.rs` (the `#[cfg(feature = "rocks")]` block around line 786)

**Background:** The `secondary_index_cf_options` function has the merge operator commented out. Without it, RocksDB secondary indices silently fail for duplicate/appended keys — only the last write wins instead of accumulating node IDs.

- [ ] **Step 1: Write a failing test**

In `sparrow-db/src/sparrow_engine/tests/` (or the rocks-specific test file), add:

```rust
#[cfg(feature = "rocks")]
#[test]
fn test_rocks_secondary_index_accumulates_multiple_ids() {
    // Create two nodes with the same secondary-indexed property value.
    // Query by that value — both IDs must appear.
    let (storage, _dir) = test_utils::create_rocks_test_storage();
    let mut txn = storage.write_txn().unwrap();

    storage.create_secondary_index("age").unwrap();

    // Add two nodes with age = 30
    let id1 = storage.add_node(&mut txn, "person", Some(&[("age", Value::I64(30))]), &arena).unwrap();
    let id2 = storage.add_node(&mut txn, "person", Some(&[("age", Value::I64(30))]), &arena).unwrap();
    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let results = storage.get_by_secondary_index(&txn, "age", &Value::I64(30), &arena).unwrap();
    assert_eq!(results.len(), 2, "secondary index must return both nodes");
    assert!(results.iter().any(|n| n.id == id1));
    assert!(results.iter().any(|n| n.id == id2));
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test --features rocks test_rocks_secondary_index_accumulates_multiple_ids 2>&1 | tail -15
```

- [ ] **Step 3: Implement and enable the merge operator**

In `sparrow-db/src/sparrow_engine/storage_core/mod.rs`, find the commented-out block (line ~786). Uncomment and fix the `merge_append` function, then enable it:

```rust
pub fn secondary_index_cf_options() -> rocksdb::Options {
    let mut opts = rocksdb::Options::default();
    opts.set_merge_operator_associative("append", Self::merge_append);
    opts
}

fn merge_append(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &rocksdb::MergeOperands,
) -> Option<Vec<u8>> {
    let mut result = existing.map(|v| v.to_vec()).unwrap_or_default();
    for op in operands {
        // Each operand is a 16-byte node ID — deduplicate
        if op.len() == 16 && !result.chunks(16).any(|chunk| chunk == op) {
            result.extend_from_slice(op);
        }
    }
    Some(result)
}
```

> **Note:** The merge operator must be set when the CF is *opened*, not just when it's created. Verify that `secondary_index_cf_options` is called both on initial creation and on every subsequent open of the database. If there's a separate open-CF path, update it too.

- [ ] **Step 4: Update secondary index writes to use `merge` instead of `put`**

Find where secondary index entries are written (grep for `secondary_indices` and `put` in the rocks storage code). Change from `put` to `merge` so the append operator is invoked:

```rust
// Before:
txn.put_cf(&cf, &index_key, &node_id_bytes)?;
// After:
txn.merge_cf(&cf, &index_key, &node_id_bytes)?;
```

- [ ] **Step 5: Run tests**

```bash
cargo test --features rocks test_rocks_secondary_index_accumulates_multiple_ids 2>&1 | tail -10
cargo check --workspace 2>&1 | tail -5
```

- [ ] **Step 6: Commit**

```bash
git add sparrow-db/src/sparrow_engine/storage_core/mod.rs
git commit -m "fix(rocks): enable secondary index merge operator — fixes silent key overwrite under RocksDB"
```

---

## Task 5: `GET /diagnostics` Endpoint

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` — add `stats()` method
- Modify: `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_core.rs` — add `stats()`
- Create: `sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs`
- Modify: `sparrow-db/src/sparrow_gateway/builtin/mod.rs`

**Response shape:**
```json
{
  "nodes": 1234,
  "edges": 567,
  "vectors": {
    "total": 100,
    "active": 90,
    "soft_deleted": 10,
    "hnsw_edges": 500,
    "entry_point_present": true
  }
}
```

- [ ] **Step 1: Write the failing test**

In `sparrow-db/src/sparrow_engine/tests/traversal_tests/vector_traversal_tests.rs`:

```rust
#[test]
fn test_vector_stats_counts_correctly() {
    let (storage, _dir) = test_utils::create_test_storage();
    let arena = bumpalo::Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let v1 = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[1.0, 0.0, 0.0], None, &arena)
        .unwrap();
    let _v2 = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[0.0, 1.0, 0.0], None, &arena)
        .unwrap();
    storage.vectors.delete(&mut txn, v1.id, &arena).unwrap();

    let stats = storage.vectors.stats(&txn).unwrap();
    assert_eq!(stats.total, 2);
    assert_eq!(stats.active, 1);
    assert_eq!(stats.soft_deleted, 1);
    assert!(stats.entry_point_present); // entry point was reassigned after v1 soft-delete

    txn.commit().unwrap();
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test test_vector_stats_counts_correctly 2>&1 | tail -15
```

- [ ] **Step 3: Add `VectorStats` struct and `stats()` to LMDB VectorCore**

In `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs`:

```rust
pub struct VectorStats {
    pub total: u64,
    pub active: u64,
    pub soft_deleted: u64,
    pub hnsw_edges: u64,
    pub entry_point_present: bool,
}

impl VectorCore {
    pub fn stats<'db>(&self, txn: &RoTxn<'db>) -> Result<VectorStats, VectorError> {
        let mut total: u64 = 0;
        let mut soft_deleted: u64 = 0;

        let iter = self.vector_properties_db.iter(txn)?;
        for result in iter {
            let (_id, bytes) = result?;
            // Minimal parse: check deleted flag without full deserialization
            // VectorWithoutData bincode layout has `deleted: bool` — parse to check
            let props: VectorWithoutData = bincode::deserialize(bytes)
                .map_err(|e| VectorError::VectorCoreError(e.to_string()))?;
            total += 1;
            if props.deleted { soft_deleted += 1; }
        }

        let hnsw_edges = self.edges_db.len(txn)? as u64;

        let entry_point_present = self.vectors_db.get(txn, ENTRY_POINT_KEY)?.is_some();

        Ok(VectorStats {
            total,
            active: total.saturating_sub(soft_deleted),
            soft_deleted,
            hnsw_edges,
            entry_point_present,
        })
    }
}
```

> **Note:** `VectorWithoutData::from_bincode_bytes` requires an arena. Use `bincode::deserialize` directly if the struct derives `Deserialize` without lifetime parameters, or use a stack-allocated arena.

- [ ] **Step 4: Add `stats()` to RocksDB VectorCore**

Apply the equivalent implementation using `self.db.iterator_cf_opt` on the `vector_data` CF. Count total entries, check `deleted` field in each.

- [ ] **Step 5: Create `sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs`**

```rust
use std::sync::Arc;
use axum::{body::Body, extract::State, response::IntoResponse};
use sonic_rs::json;
use sparrow_db::sparrow_engine::storage_core::txn::ReadTransaction;
use crate::{
    protocol::{self, request::RequestType, Format},
    sparrow_engine::{storage_core::storage_methods::StorageMethods, types::GraphError},
    sparrow_gateway::{
        gateway::AppState,
        router::router::{Handler, HandlerInput, HandlerSubmission},
    },
};

pub async fn diagnostics_handler(
    State(state): State<Arc<AppState>>,
) -> axum::http::Response<Body> {
    let req = protocol::request::Request {
        name: "diagnostics".to_string(),
        req_type: RequestType::Query,
        api_key_hash: None,
        body: axum::body::Bytes::new(),
        in_fmt: Format::default(),
        out_fmt: Format::default(),
    };
    match state.worker_pool.process(req).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            let body = sonic_rs::to_string(&json!({"error": e.to_string()})).unwrap_or_default();
            axum::http::Response::builder()
                .status(500)
                .body(Body::from(body))
                .unwrap()
        }
    }
}

pub fn diagnostics_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let graph = &input.graph;
    let storage = graph.storage.read().unwrap();

    #[cfg(feature = "lmdb")]
    let txn = storage.graph_env.read_txn().map_err(|e| GraphError::from(e))?;
    #[cfg(feature = "rocks")]
    let txn = storage.read_txn()?;

    let node_count = storage.nodes_db.len(&txn).unwrap_or(0) as u64;
    let edge_count = storage.edges_db.len(&txn).unwrap_or(0) as u64;
    let vstats = storage.vectors.stats(&txn)
        .map_err(|e| GraphError::from(e))?;

    let body = sonic_rs::to_vec(&json!({
        "nodes": node_count,
        "edges": edge_count,
        "vectors": {
            "total": vstats.total,
            "active": vstats.active,
            "soft_deleted": vstats.soft_deleted,
            "hnsw_edges": vstats.hnsw_edges,
            "entry_point_present": vstats.entry_point_present,
        }
    })).map_err(|e| GraphError::Other(e.to_string()))?;

    Ok(protocol::Response {
        body: axum::body::Bytes::from(body),
        ..Default::default()
    })
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("diagnostics", diagnostics_inner, false)
    )
}
```

> **Note:** Adapt node_count/edge_count to actual field names and APIs available on `SparrowGraphStorage`. For RocksDB, use `self.db.iterator_cf_opt` to count. Check existing builtins like `all_nodes_and_edges.rs` for the exact pattern used to read counts.

- [ ] **Step 6: Register the handler in `sparrow-db/src/sparrow_gateway/builtin/mod.rs`**

```rust
pub mod diagnostics;
```

- [ ] **Step 7: Verify the handler appears in the route list**

```bash
cargo check --workspace 2>&1 | tail -5
```

Run the server locally (if permitted — only `cargo check` without build): check that `"diagnostics"` appears in the inventory. Since we can't run the binary, at minimum confirm `cargo check` passes.

- [ ] **Step 8: Commit**

```bash
git add sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs \
        sparrow-db/src/sparrow_gateway/builtin/mod.rs \
        sparrow-db/src/sparrow_engine/vector_core/
git commit -m "feat(diagnostics): add GET /diagnostics endpoint — node, edge, vector counts + entry point health"
```

---

## Task 6: `POST /vector-soft-delete` and `POST /vector-hard-delete` Endpoints

**Files:**
- Create: `sparrow-db/src/sparrow_gateway/builtin/vector_ops.rs`
- Modify: `sparrow-db/src/sparrow_gateway/builtin/mod.rs`

Both endpoints accept `{ "id": "<uuid-string>" }` and are `is_write: true`.

- [ ] **Step 1: Write the failing test**

In `sparrow-db/src/sparrow_gateway/builtin/` (or the gateway integration test), write:

```rust
#[test]
fn test_vector_soft_delete_handler_returns_ok() {
    // Setup a test engine, insert a vector, call the handler with its id
    // Verify: response is 200, vector is soft-deleted in storage
    // (use the same pattern as existing handler tests)
}

#[test]
fn test_vector_hard_delete_handler_removes_all_data() {
    // Setup a test engine, insert a vector, call hard delete handler
    // Verify: vectors_db has no entries for that id
}
```

Look at `sparrow-db/src/sparrow_gateway/builtin/node_by_id.rs` for the test pattern and replicate it.

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test test_vector_soft_delete_handler_returns_ok test_vector_hard_delete_handler_removes_all_data 2>&1 | tail -15
```

- [ ] **Step 3: Create `vector_ops.rs`**

```rust
use std::sync::Arc;
use axum::{body::Body, extract::State, response::IntoResponse};
use sonic_rs::{JsonValueTrait, json};
use crate::{
    protocol::{self, request::RequestType, Format},
    sparrow_engine::{
        storage_core::{storage_methods::StorageMethods, txn::WriteTransaction},
        traversal_core::SparrowGraphEngine,
        types::GraphError,
        vector_core::lmdb::hnsw::HNSW,
    },
    sparrow_gateway::{
        gateway::AppState,
        router::router::{Handler, HandlerInput, HandlerSubmission},
    },
    utils::id::parse_uuid_str,
};

// ── /vector-soft-delete ────────────────────────────────────────────────────

pub async fn vector_soft_delete_handler(
    State(state): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> axum::http::Response<Body> {
    let req = protocol::request::Request {
        name: "vector_soft_delete".to_string(),
        req_type: RequestType::Mutation,
        api_key_hash: None,
        body,
        in_fmt: Format::default(),
        out_fmt: Format::default(),
    };
    match state.worker_pool.process(req).await {
        Ok(r) => r.into_response(),
        Err(e) => axum::http::Response::builder()
            .status(500)
            .body(Body::from(e.to_string()))
            .unwrap(),
    }
}

pub fn vector_soft_delete_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let body: sonic_rs::Value = sonic_rs::from_slice(&input.request.body)
        .map_err(|e| GraphError::Other(format!("invalid JSON: {e}")))?;
    let id_str = body["id"].as_str()
        .ok_or_else(|| GraphError::Other("missing field: id".into()))?;
    let id = parse_uuid_str(id_str)
        .map_err(|e| GraphError::Other(format!("invalid uuid: {e}")))?;

    let graph = &input.graph;
    let storage = graph.storage.write().unwrap();
    let mut txn = storage.write_txn()?;
    let arena = bumpalo::Bump::new();

    storage.vectors.delete(&mut txn, id, &arena)
        .map_err(|e| GraphError::from(e))?;
    txn.commit().map_err(|e| GraphError::from(e))?;

    let body = sonic_rs::to_vec(&json!({"ok": true, "id": id_str}))
        .map_err(|e| GraphError::Other(e.to_string()))?;
    Ok(protocol::Response {
        body: axum::body::Bytes::from(body),
        ..Default::default()
    })
}

inventory::submit! {
    HandlerSubmission(Handler::new("vector_soft_delete", vector_soft_delete_inner, true))
}

// ── /vector-hard-delete ────────────────────────────────────────────────────

pub async fn vector_hard_delete_handler(
    State(state): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> axum::http::Response<Body> {
    let req = protocol::request::Request {
        name: "vector_hard_delete".to_string(),
        req_type: RequestType::Mutation,
        api_key_hash: None,
        body,
        in_fmt: Format::default(),
        out_fmt: Format::default(),
    };
    match state.worker_pool.process(req).await {
        Ok(r) => r.into_response(),
        Err(e) => axum::http::Response::builder()
            .status(500)
            .body(Body::from(e.to_string()))
            .unwrap(),
    }
}

pub fn vector_hard_delete_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let body: sonic_rs::Value = sonic_rs::from_slice(&input.request.body)
        .map_err(|e| GraphError::Other(format!("invalid JSON: {e}")))?;
    let id_str = body["id"].as_str()
        .ok_or_else(|| GraphError::Other("missing field: id".into()))?;
    let id = parse_uuid_str(id_str)
        .map_err(|e| GraphError::Other(format!("invalid uuid: {e}")))?;

    let graph = &input.graph;
    let storage = graph.storage.write().unwrap();
    let mut txn = storage.write_txn()?;

    storage.vectors.hard_delete(&mut txn, id)
        .map_err(|e| GraphError::from(e))?;
    txn.commit().map_err(|e| GraphError::from(e))?;

    let body = sonic_rs::to_vec(&json!({"ok": true, "id": id_str, "warning": "HNSW graph is structurally degraded; consider POST /rebuild-vector-index"}))
        .map_err(|e| GraphError::Other(e.to_string()))?;
    Ok(protocol::Response {
        body: axum::body::Bytes::from(body),
        ..Default::default()
    })
}

inventory::submit! {
    HandlerSubmission(Handler::new("vector_hard_delete", vector_hard_delete_inner, true))
}
```

> **Note:** Find the actual `parse_uuid_str` or equivalent utility used in this codebase by grepping for `uuid` in the existing builtins. Adapt accordingly.

- [ ] **Step 4: Add to `mod.rs`**

```rust
pub mod vector_ops;
```

- [ ] **Step 5: Run tests and check**

```bash
cargo test test_vector_soft_delete_handler test_vector_hard_delete_handler 2>&1 | tail -15
cargo check --workspace 2>&1 | tail -5
```

- [ ] **Step 6: Commit**

```bash
git add sparrow-db/src/sparrow_gateway/builtin/
git commit -m "feat(vector): add POST /vector-soft-delete and POST /vector-hard-delete endpoints"
```

---

## Task 7: `insert_with_id`, `POST /rebuild-vector-index`, `POST /purge-soft-deleted`

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/hnsw.rs` — add `insert_with_id` to trait
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` — implement `insert_with_id`, `rebuild`, `purge_soft_deleted`
- Modify: `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_core.rs` — same
- Create: `sparrow-db/src/sparrow_gateway/builtin/vector_rebuild.rs`
- Modify: `sparrow-db/src/sparrow_gateway/builtin/mod.rs`

**Design:** `insert_with_id` is a variant of `insert` that accepts a caller-specified `u128` instead of calling `v6_uuid()`. This preserves external ID references across a rebuild.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn test_insert_with_id_preserves_id() {
    let (storage, _dir) = test_utils::create_test_storage();
    let arena = bumpalo::Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let fixed_id: u128 = 0xdeadbeef_cafebabe_12345678_90abcdef;
    let inserted = storage.vectors
        .insert_with_id::<fn(&_, &_) -> bool>(
            &mut txn, fixed_id, "test", &[1.0, 0.0, 0.0], None, &arena
        )
        .expect("insert_with_id failed");

    assert_eq!(inserted.id, fixed_id, "id must be preserved");
    txn.commit().unwrap();
}

#[test]
fn test_rebuild_preserves_active_vectors_and_purges_deleted() {
    let (storage, _dir) = test_utils::create_test_storage();
    let arena = bumpalo::Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let v1 = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[1.0, 0.0, 0.0], None, &arena).unwrap();
    let v2 = storage.vectors
        .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[0.0, 1.0, 0.0], None, &arena).unwrap();
    storage.vectors.delete(&mut txn, v1.id, &arena).unwrap(); // soft-delete v1

    let stats = storage.vectors.rebuild(&mut txn, &arena).expect("rebuild failed");
    assert_eq!(stats.kept, 1);
    assert_eq!(stats.purged_deleted, 1);

    // v2 must still be findable with original id
    let found = storage.vectors.get_full_vector(&txn, v2.id, &arena);
    assert!(found.is_ok(), "active vector must survive rebuild");

    // v1 must be gone
    let deleted_props = storage.vectors.vector_properties_db.get(&txn, &v1.id).unwrap();
    assert!(deleted_props.is_none(), "deleted vector must be purged after rebuild");

    txn.commit().unwrap();
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test test_insert_with_id_preserves_id test_rebuild_preserves_active_vectors_and_purges_deleted 2>&1 | tail -20
```

- [ ] **Step 3: Add `insert_with_id` to HNSW trait**

In `sparrow-db/src/sparrow_engine/vector_core/lmdb/hnsw.rs`:

```rust
fn insert_with_id<'db, 'arena, 'txn, F>(
    &'db self,
    txn: &'txn mut RwTxn<'db>,
    id: u128,
    label: &'arena str,
    data: &'arena [f64],
    properties: Option<ImmutablePropertiesMap<'arena>>,
    arena: &'arena bumpalo::Bump,
) -> Result<HVector<'arena>, VectorError>
where
    F: Fn(&HVector<'arena>, &RoTxn<'db>) -> bool,
    'db: 'arena,
    'arena: 'txn;
```

- [ ] **Step 4: Implement `insert_with_id` on LMDB VectorCore**

Copy the body of `fn insert` verbatim, then change the ID generation line:

```rust
// In insert(): let new_id = v6_uuid();
// In insert_with_id(): use the caller-supplied id directly
let mut query = HVector::from_slice(label, 0, data);
query.id = id;  // Override the default id
query.properties = properties;
```

The rest of the insert logic (level selection, HNSW edge creation, entry point handling) is identical.

- [ ] **Step 5: Add `RebuildStats` and `rebuild` to LMDB VectorCore**

```rust
pub struct RebuildStats {
    pub kept: u64,
    pub purged_deleted: u64,
}

pub fn rebuild(&self, txn: &mut RwTxn, arena: &bumpalo::Bump) -> Result<RebuildStats, VectorError> {
    // Phase 1: Collect all non-deleted vectors (owned data, so txn can be reused for writes)
    let mut to_reinsert: Vec<(u128, Vec<f64>, String, Option<Vec<u8>>)> = Vec::new();
    let mut purged: u64 = 0;

    {
        let iter = self.vector_properties_db.iter(txn)?;
        for result in iter {
            let (id_key, props_bytes) = result?;
            let props: VectorWithoutData = bincode::deserialize(props_bytes)
                .map_err(|e| VectorError::VectorCoreError(e.to_string()))?;
            if props.deleted {
                purged += 1;
                continue;
            }
            // Fetch raw vector data at level 0
            let data_bytes = self.vectors_db.get(txn, &Self::vector_key(*id_key, 0))?
                .map(|b| b.to_vec())
                .ok_or(VectorError::VectorNotFound(id_key.to_string()))?;
            let data_f64: Vec<f64> = bytemuck::cast_slice(&data_bytes).to_vec();
            to_reinsert.push((*id_key, data_f64, props.label.to_string(), None));
        }
    }

    // Phase 2: Clear all three vector tables
    // Clear vectors_db (all v: prefixed entries + entry_point)
    let all_vector_keys: Vec<Vec<u8>> = self.vectors_db
        .iter(txn)?
        .filter_map(|r| r.ok().map(|(k, _)| k.to_vec()))
        .collect();
    for k in all_vector_keys { self.vectors_db.delete(txn, k.as_ref())?; }

    // Clear edges_db
    let all_edge_keys: Vec<Vec<u8>> = self.edges_db
        .iter(txn)?
        .filter_map(|r| r.ok().map(|(k, _)| k.to_vec()))
        .collect();
    for k in all_edge_keys { self.edges_db.delete(txn, k.as_ref())?; }

    // Clear vector_properties_db
    let all_prop_keys: Vec<u128> = self.vector_properties_db
        .iter(txn)?
        .filter_map(|r| r.ok().map(|(k, _)| *k))
        .collect();
    for k in all_prop_keys { self.vector_properties_db.delete(txn, &k)?; }

    // Phase 3: Re-insert each active vector with its original ID
    let kept = to_reinsert.len() as u64;
    for (id, data, label, _props) in to_reinsert {
        let data_arena = arena.alloc_slice_copy(&data);
        self.insert_with_id::<fn(&_, &_) -> bool>(txn, id, &label, data_arena, None, arena)?;
    }

    Ok(RebuildStats { kept, purged_deleted: purged })
}

pub fn purge_soft_deleted(&self, txn: &mut RwTxn, label_filter: Option<&str>, arena: &bumpalo::Bump) -> Result<RebuildStats, VectorError> {
    // If no label filter, a full rebuild already purges soft-deleted vectors.
    // With a label filter: collect non-deleted for the given label only, rebuild index for that label.
    // For simplicity in V1, purge_soft_deleted is a thin alias over rebuild() — label filter is a future enhancement.
    if label_filter.is_some() {
        // TODO: label-scoped rebuild
        return Err(VectorError::VectorCoreError("label-scoped purge not yet implemented; omit label to purge all".into()));
    }
    self.rebuild(txn, arena)
}
```

> **Note:** `bytemuck::cast_slice` requires `bytemuck` as a dependency. Check if it's already in `Cargo.toml` — if not, the bytes-to-f64 cast can be done with:
> ```rust
> let data_f64: Vec<f64> = data_bytes.chunks_exact(8)
>     .map(|b| f64::from_be_bytes(b.try_into().unwrap()))
>     .collect();
> ```
> Check how `put_vector` serializes data to determine the endianness.

- [ ] **Step 6: Apply equivalent to RocksDB VectorCore**

Implement `insert_with_id`, `rebuild`, and `purge_soft_deleted` in `rocks/vector_core.rs` using the same logic adapted for RocksDB column families.

- [ ] **Step 7: Create `vector_rebuild.rs` handler**

```rust
// POST /rebuild-vector-index — long-running, is_write: true
pub fn rebuild_vector_index_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let graph = &input.graph;
    let storage = graph.storage.write().unwrap();
    let mut txn = storage.write_txn()?;
    let arena = bumpalo::Bump::new();

    let stats = storage.vectors.rebuild(&mut txn, &arena)
        .map_err(|e| GraphError::from(e))?;
    txn.commit().map_err(|e| GraphError::from(e))?;

    let body = sonic_rs::to_vec(&json!({
        "ok": true,
        "kept": stats.kept,
        "purged_deleted": stats.purged_deleted,
    })).map_err(|e| GraphError::Other(e.to_string()))?;
    Ok(protocol::Response { body: axum::body::Bytes::from(body), ..Default::default() })
}

inventory::submit! {
    HandlerSubmission(Handler::new("rebuild_vector_index", rebuild_vector_index_inner, true))
}

// POST /purge-soft-deleted — optional body { "label": "..." }
pub fn purge_soft_deleted_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let label_filter: Option<String> = if input.request.body.is_empty() {
        None
    } else {
        let body: sonic_rs::Value = sonic_rs::from_slice(&input.request.body)
            .map_err(|e| GraphError::Other(format!("invalid JSON: {e}")))?;
        body["label"].as_str().map(|s| s.to_string())
    };

    let graph = &input.graph;
    let storage = graph.storage.write().unwrap();
    let mut txn = storage.write_txn()?;
    let arena = bumpalo::Bump::new();

    let stats = storage.vectors
        .purge_soft_deleted(&mut txn, label_filter.as_deref(), &arena)
        .map_err(|e| GraphError::from(e))?;
    txn.commit().map_err(|e| GraphError::from(e))?;

    let body = sonic_rs::to_vec(&json!({
        "ok": true,
        "purged": stats.purged_deleted,
        "remaining": stats.kept,
    })).map_err(|e| GraphError::Other(e.to_string()))?;
    Ok(protocol::Response { body: axum::body::Bytes::from(body), ..Default::default() })
}

inventory::submit! {
    HandlerSubmission(Handler::new("purge_soft_deleted", purge_soft_deleted_inner, true))
}
```

- [ ] **Step 8: Register in `mod.rs`**

```rust
pub mod vector_rebuild;
```

- [ ] **Step 9: Run all tests**

```bash
cargo test test_insert_with_id_preserves_id test_rebuild_preserves_active_vectors_and_purges_deleted 2>&1 | tail -20
cargo check --workspace 2>&1 | tail -5
```

- [ ] **Step 10: Commit**

```bash
git add sparrow-db/src/sparrow_engine/vector_core/ sparrow-db/src/sparrow_gateway/builtin/
git commit -m "feat(vector): add insert_with_id, rebuild_vector_index, purge_soft_deleted — closes HNSW rebuild gap"
```

---

## Self-Review

**Spec coverage:**

| Item | Task | Status |
|---|---|---|
| DROP leak: `hard_delete` wired into `drop_vector` | 1 | ✓ |
| Entry point cleared on hard delete | 1 | ✓ |
| Entry point reassigned on soft delete | 2 | ✓ |
| `Value` arithmetic overflow (Add, Sub, Mul, Div, Rem) | 3 | ✓ |
| RocksDB secondary index merge operator | 4 | ✓ |
| `GET /diagnostics` | 5 | ✓ |
| `POST /vector-soft-delete` | 6 | ✓ |
| `POST /vector-hard-delete` | 6 | ✓ |
| `insert_with_id` | 7 | ✓ |
| `POST /rebuild-vector-index` | 7 | ✓ |
| `POST /purge-soft-deleted` | 7 | ✓ |

**Level type inconsistency (usize LMDB vs u8 RocksDB):** Intentionally deferred. Changing the LMDB key format is a data-migration breaking change. In practice, HNSW level distributions are logarithmic — graphs with more than 20 levels require tens of billions of vectors. This is not a practical stability concern today. Document it with a `TODO` comment if encountered.

**What this plan does NOT include:**
- `GET /check-duplicates` — deferred (P3, no stability impact)
- LMDB level type migration — deferred (data-migration risk, near-zero practical impact)
