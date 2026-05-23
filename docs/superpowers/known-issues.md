# SparrowDB — Known Issues & Workarounds

This file tracks non-trivial bugs or behavioural quirks discovered during
development that have been worked around rather than fixed at the source.
Each entry records the root cause, the affected code path, and the chosen
workaround so future engineers understand why the code looks the way it does.

---

## 1. `PutFlags::APPEND` + non-monotonic v6 UUIDs causes `MDB_KEYEXIST`

**Status:** Worked around in `crates/sparrow-benches/src/lib.rs`. Root cause
in `add_n` / `add_edge` is unfixed.

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

### Potential permanent fix

The correct long-term fix is to remove `PutFlags::APPEND` from `add_n` and
`add_edge` and replace it with a plain `put()`.  `APPEND` was likely added
as a performance optimisation based on the assumption that UUIDs are always
generated in ascending order, but this assumption breaks under load.
The performance gain from `APPEND` is small (it skips a B-tree search) and
not worth the fragility.

**Files to change for the permanent fix:**

| File | Lines | Change |
|------|-------|--------|
| `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_n.rs` | ~69, ~174 | `PutFlags::APPEND` → plain `put()` |
| `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_e.rs` | ~112 | `PutFlags::APPEND` → plain `put()` |

`APPEND_DUP` on the out/in-edge index databases is unaffected — within a
single key the values *are* written in ascending order (edge data is
`edge_id || node_id` and edge IDs are generated one at a time, so there is
never a second value for the same key in a single request). `APPEND_DUP`
can stay.

---

## 2. (Placeholder for future issues)

_No additional issues recorded._
