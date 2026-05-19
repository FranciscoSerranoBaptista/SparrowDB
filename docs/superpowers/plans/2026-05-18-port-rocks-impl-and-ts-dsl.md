# Port rocks-impl and python-typescript-dsl Branches Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the dual-storage-backend abstraction (LMDB + RocksDB via compile-time feature flags) and the TypeScript query IR from two upstream branches into this pre-v2 snapshot.

**Architecture:** Cargo feature flags (`lmdb` / `rocks`) select the backend at compile time. Transaction types are aliased to `RTxn`/`WTxn` in `traversal_core/mod.rs` so all traversal ops remain backend-agnostic. VectorCore and BM25 have parallel implementations under `vector_core/lmdb/` and `vector_core/rocks/`. The storage core wraps each backend in a namespaced `pub mod lmdb {}` / `pub mod rocks {}` module with a `pub use <backend>::*` re-export. The TypeScript IR in `helix-ts/` is a standalone Deno project defining the query DSL type system.

**Tech Stack:** Rust, heed3 (LMDB), rocksdb crate, Deno/TypeScript, gh CLI (for fetching upstream files)

**Upstream source:** `https://github.com/HelixDB/helix-db`
- Storage branch: `rocks-impl`
- TypeScript branch: `python-typescript-dsl`

---

### Task 1: Copy TypeScript/Python DSL files

**Files:**
- Create: `helix-ts/ir.ts`
- Create: `helix-ts/main.ts`
- Create: `helix-ts/deno.json`
- Create: `examples/bookstore.py`
- Create: `examples/bookstore.ts`

- [ ] **Step 1: Fetch and write all five files**

```bash
mkdir -p helix-ts examples

gh api "repos/HelixDB/helix-db/contents/helix-ts/ir.ts?ref=python-typescript-dsl" \
  --jq '.content' | base64 -d > helix-ts/ir.ts

gh api "repos/HelixDB/helix-db/contents/helix-ts/main.ts?ref=python-typescript-dsl" \
  --jq '.content' | base64 -d > helix-ts/main.ts

gh api "repos/HelixDB/helix-db/contents/helix-ts/deno.json?ref=python-typescript-dsl" \
  --jq '.content' | base64 -d > helix-ts/deno.json

gh api "repos/HelixDB/helix-db/contents/examples/bookstore.py?ref=python-typescript-dsl" \
  --jq '.content' | base64 -d > examples/bookstore.py

gh api "repos/HelixDB/helix-db/contents/examples/bookstore.ts?ref=python-typescript-dsl" \
  --jq '.content' | base64 -d > examples/bookstore.ts
```

- [ ] **Step 2: Verify files landed**

```bash
ls -la helix-ts/ examples/
```

Expected: `ir.ts`, `main.ts`, `deno.json` in `helix-ts/`; `bookstore.py`, `bookstore.ts` in `examples/`.

- [ ] **Step 3: Commit**

```bash
git add helix-ts/ examples/
git commit -m "feat: add TypeScript query IR and SDK examples from python-typescript-dsl"
```

---

### Task 2: Add Cargo feature flags

**Files:**
- Modify: `helix-db/Cargo.toml`

- [ ] **Step 1: Replace the `[dependencies]` heed3 entry and add rocksdb + features**

In `helix-db/Cargo.toml`, find the existing `heed3` dependency line and replace it. Then update the `[features]` section to match:

```toml
heed3 = { version = "0.22.0", optional = true }
rocksdb = { version = "0.24.0", features = ["multi-threaded-cf"], optional = true }
```

Replace the entire `[features]` block with:

```toml
[features]
debug-output = ["helix-macros/debug-output"]
compiler = ["pest", "pest_derive"]
cosine = []
api-key = []
build = ["compiler"]
vectors = ["cosine", "url"]
server = ["build", "compiler", "vectors", "reqwest"]
full = ["build", "compiler", "vectors"]
bench = ["polars"]
dev = ["debug-output", "server", "bench"]
dev-instance = []
lmdb = ["server", "heed3"]
rocks = ["server", "rocksdb"]
default = ["lmdb"]
production = ["api-key"]
```

Note: `default = ["lmdb"]` keeps the current behaviour. Switch to `default = ["rocks"]` once the RocksDB port is verified.

- [ ] **Step 2: Verify the toml parses**

```bash
cd helix-db && cargo metadata --no-deps --quiet 2>&1 | head -5
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add helix-db/Cargo.toml
git commit -m "feat: add lmdb/rocks optional cargo features"
```

---

### Task 3: Transaction type aliases and txn.rs

**Files:**
- Create: `helix-db/src/helix_engine/storage_core/txn.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/mod.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/traversal_iter.rs`

- [ ] **Step 1: Create `storage_core/txn.rs`**

```rust
// helix-db/src/helix_engine/storage_core/txn.rs
use crate::helix_engine::{traversal_core::{RTxn, WTxn}, types::GraphError};

pub trait ReadTransaction {
    fn read_txn(&self) -> Result<RTxn<'_>, GraphError>;
}

pub trait WriteTransaction {
    fn write_txn(&self) -> Result<WTxn<'_>, GraphError>;
}

#[cfg(feature = "rocks")]
use std::sync::Arc;

#[cfg(feature = "rocks")]
impl ReadTransaction for Arc<rocksdb::TransactionDB<rocksdb::MultiThreaded>> {
    fn read_txn(&self) -> Result<RTxn<'_>, GraphError> {
        Ok(self.transaction())
    }
}

#[cfg(feature = "rocks")]
impl WriteTransaction for Arc<rocksdb::TransactionDB<rocksdb::MultiThreaded>> {
    fn write_txn(&self) -> Result<WTxn<'_>, GraphError> {
        Ok(self.transaction())
    }
}
```

- [ ] **Step 2: Add `RTxn`/`WTxn` type aliases to `traversal_core/mod.rs`**

Append to the end of `helix-db/src/helix_engine/traversal_core/mod.rs`:

```rust
#[cfg(feature = "rocks")]
pub type WTxn<'db> = rocksdb::Transaction<'db, rocksdb::TransactionDB>;
#[cfg(feature = "rocks")]
pub type RTxn<'db> = rocksdb::Transaction<'db, rocksdb::TransactionDB>;

#[cfg(feature = "lmdb")]
pub type WTxn<'db> = heed3::RwTxn<'db>;
#[cfg(feature = "lmdb")]
pub type RTxn<'db> = heed3::RoTxn<'db>;
```

- [ ] **Step 3: Update `traversal_iter.rs` to use `RTxn`/`WTxn`**

Replace the imports at the top of `helix-db/src/helix_engine/traversal_core/traversal_iter.rs`:

Old:
```rust
use crate::{
    helix_engine::{
        storage_core::HelixGraphStorage, traversal_core::traversal_value::TraversalValue,
        types::GraphError,
    },
    protocol::value::Value,
};
use heed3::{RoTxn, RwTxn};
```

New:
```rust
use crate::{
    helix_engine::{
        storage_core::HelixGraphStorage,
        traversal_core::{RTxn, WTxn, traversal_value::TraversalValue},
        types::GraphError,
    },
    protocol::value::Value,
};
```

Then replace every `RoTxn<'db>` with `RTxn<'db>` and every `RwTxn<'db>` with `WTxn<'db>` throughout the file.

- [ ] **Step 4: Add `pub mod txn` to `storage_core/mod.rs`**

In `helix-db/src/helix_engine/storage_core/mod.rs`, add to the module declarations at the top:

```rust
pub mod txn;
```

- [ ] **Step 5: Verify it compiles with lmdb feature**

```bash
cd helix-db && cargo check --features lmdb 2>&1 | head -30
```

Expected: no new errors beyond pre-existing ones.

- [ ] **Step 6: Commit**

```bash
git add helix-db/src/helix_engine/storage_core/txn.rs \
        helix-db/src/helix_engine/traversal_core/mod.rs \
        helix-db/src/helix_engine/traversal_core/traversal_iter.rs \
        helix-db/src/helix_engine/storage_core/mod.rs
git commit -m "feat: abstract transaction types behind RTxn/WTxn aliases"
```

---

### Task 4: Feature-flag the StorageMethods trait

**Files:**
- Modify: `helix-db/src/helix_engine/storage_core/storage_methods.rs`

- [ ] **Step 1: Replace the file content**

```rust
// helix-db/src/helix_engine/storage_core/storage_methods.rs
use crate::helix_engine::types::GraphError;
use crate::utils::items::{Edge, Node};

pub trait DBMethods {
    fn create_secondary_index(&mut self, name: &str) -> Result<(), GraphError>;
    fn drop_secondary_index(&mut self, name: &str) -> Result<(), GraphError>;
}

#[cfg(feature = "lmdb")]
use heed3::{RoTxn, RwTxn};

#[cfg(feature = "lmdb")]
pub trait StorageMethods {
    fn get_node<'arena>(
        &self,
        txn: &RoTxn,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Node<'arena>, GraphError>;

    fn get_edge<'arena>(
        &self,
        txn: &RoTxn,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Edge<'arena>, GraphError>;

    fn drop_node(&self, txn: &mut RwTxn, id: u128) -> Result<(), GraphError>;
    fn drop_edge(&self, txn: &mut RwTxn, id: u128) -> Result<(), GraphError>;
    fn drop_vector(&self, txn: &mut RwTxn, id: u128) -> Result<(), GraphError>;
}

#[cfg(feature = "rocks")]
pub trait StorageMethods {
    fn get_node<'arena>(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Node<'arena>, GraphError>;

    fn get_edge<'arena>(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Edge<'arena>, GraphError>;

    fn drop_node(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
    ) -> Result<(), GraphError>;

    fn drop_edge(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
    ) -> Result<(), GraphError>;

    fn drop_vector(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
    ) -> Result<(), GraphError>;
}
```

- [ ] **Step 2: Check compilation**

```bash
cd helix-db && cargo check --features lmdb 2>&1 | grep "^error" | head -20
```

- [ ] **Step 3: Commit**

```bash
git add helix-db/src/helix_engine/storage_core/storage_methods.rs
git commit -m "feat: feature-flag StorageMethods trait for lmdb/rocks"
```

---

### Task 5: Reorganise vector_core into lmdb/ subdirectory

**Files:**
- Create: `helix-db/src/helix_engine/vector_core/lmdb/mod.rs`
- Move (copy + delete): `vector_core/{binary_heap,hnsw,utils,vector_core,vector_distance}.rs` → `vector_core/lmdb/`

- [ ] **Step 1: Create the lmdb subdirectory and move files**

```bash
cd helix-db/src/helix_engine/vector_core
mkdir -p lmdb
cp binary_heap.rs lmdb/binary_heap.rs
cp hnsw.rs lmdb/hnsw.rs
cp utils.rs lmdb/utils.rs
cp vector_core.rs lmdb/vector_core.rs
cp vector_distance.rs lmdb/vector_distance.rs
```

- [ ] **Step 2: Create `lmdb/mod.rs`**

```rust
// helix-db/src/helix_engine/vector_core/lmdb/mod.rs
pub mod binary_heap;
pub mod hnsw;
pub mod utils;
pub mod vector_core;
pub mod vector_distance;
```

- [ ] **Step 3: Delete the now-redundant top-level files**

```bash
cd helix-db/src/helix_engine/vector_core
rm binary_heap.rs hnsw.rs utils.rs vector_core.rs vector_distance.rs
```

- [ ] **Step 4: Update `vector_core/mod.rs` to re-export from lmdb/ and stub rocks/**

Replace the content of `helix-db/src/helix_engine/vector_core/mod.rs`:

```rust
pub mod vector;
pub mod vector_without_data;

#[cfg(feature = "lmdb")]
pub mod lmdb;
#[cfg(feature = "lmdb")]
pub use lmdb::{
    hnsw::HNSW,
    vector_core::{ENTRY_POINT_KEY, HNSWConfig, VectorCore},
    vector_distance::{self, DistanceCalc},
};

// rocks module added in Task 7
```

- [ ] **Step 5: Check compilation**

```bash
cd helix-db && cargo check --features lmdb 2>&1 | grep "^error" | head -20
```

Expected: no new errors.

- [ ] **Step 6: Commit**

```bash
git add helix-db/src/helix_engine/vector_core/
git commit -m "refactor: move LMDB vector_core into vector_core/lmdb/ subdirectory"
```

---

### Task 6: Port rocks/ vector_core implementation

**Files:**
- Create: `helix-db/src/helix_engine/vector_core/rocks/mod.rs`
- Create: `helix-db/src/helix_engine/vector_core/rocks/binary_heap.rs`
- Create: `helix-db/src/helix_engine/vector_core/rocks/hnsw.rs`
- Create: `helix-db/src/helix_engine/vector_core/rocks/utils.rs`
- Create: `helix-db/src/helix_engine/vector_core/rocks/vector_core.rs`
- Create: `helix-db/src/helix_engine/vector_core/rocks/vector_distance.rs`

- [ ] **Step 1: Fetch all rocks vector_core files from upstream**

```bash
BASE="helix-db/src/helix_engine/vector_core/rocks"
mkdir -p $BASE

for f in mod.rs binary_heap.rs hnsw.rs utils.rs vector_core.rs vector_distance.rs; do
  gh api "repos/HelixDB/helix-db/contents/$BASE/$f?ref=rocks-impl" \
    --jq '.content' | base64 -d > "$BASE/$f"
  echo "wrote $BASE/$f"
done
```

- [ ] **Step 2: Verify files are non-empty**

```bash
wc -l helix-db/src/helix_engine/vector_core/rocks/*.rs
```

Expected: `binary_heap.rs` ~567 lines, `vector_core.rs` ~786 lines, `vector_distance.rs` ~157 lines.

- [ ] **Step 3: Update `vector_core/mod.rs` to include rocks re-exports**

Append to `helix-db/src/helix_engine/vector_core/mod.rs`:

```rust
#[cfg(feature = "rocks")]
pub mod rocks;
#[cfg(feature = "rocks")]
pub use rocks::{
    hnsw::HNSW,
    vector_core::{HNSWConfig, VectorCore},
    vector_distance::{self, DistanceCalc},
};
```

- [ ] **Step 4: Check rocks compilation**

```bash
cd helix-db && cargo check --features rocks 2>&1 | grep "^error" | head -30
```

Fix any import path issues — the rocks files reference `crate::helix_engine::storage_core::Txn` which may need adjusting (see Task 9 where `Txn` is defined in the lmdb module; for rocks it comes from `traversal_core::RTxn`).

- [ ] **Step 5: Commit**

```bash
git add helix-db/src/helix_engine/vector_core/rocks/ \
        helix-db/src/helix_engine/vector_core/mod.rs
git commit -m "feat: add RocksDB vector_core (HNSW) implementation"
```

---

### Task 7: Reorganise bm25/ into lmdb_bm25.rs + rocks_bm25.rs

**Files:**
- Rename: `helix-db/src/helix_engine/bm25/bm25.rs` → `lmdb_bm25.rs`
- Create: `helix-db/src/helix_engine/bm25/rocks_bm25.rs`
- Modify: `helix-db/src/helix_engine/bm25/mod.rs`

- [ ] **Step 1: Rename bm25.rs**

```bash
cd helix-db/src/helix_engine/bm25
mv bm25.rs lmdb_bm25.rs
```

- [ ] **Step 2: Fetch rocks_bm25.rs from upstream**

```bash
gh api "repos/HelixDB/helix-db/contents/helix-db/src/helix_engine/bm25/rocks_bm25.rs?ref=rocks-impl" \
  --jq '.content' | base64 -d > helix-db/src/helix_engine/bm25/rocks_bm25.rs
wc -l helix-db/src/helix_engine/bm25/rocks_bm25.rs
```

Expected: ~457 lines.

- [ ] **Step 3: Replace `bm25/mod.rs`**

```rust
// helix-db/src/helix_engine/bm25/mod.rs
#[cfg(feature = "lmdb")]
pub mod lmdb_bm25;
#[cfg(feature = "rocks")]
pub mod rocks_bm25;

#[cfg(feature = "lmdb")]
pub use lmdb_bm25::{BM25, BM25Flatten, BM25Metadata, HBM25Config, HybridSearch, METADATA_KEY};
#[cfg(feature = "rocks")]
pub use rocks_bm25::{BM25, BM25Flatten, BM25Metadata, HBM25Config, HybridSearch, METADATA_KEY};

#[cfg(test)]
pub mod bm25_tests;
```

- [ ] **Step 4: Check lmdb compilation**

```bash
cd helix-db && cargo check --features lmdb 2>&1 | grep "^error" | head -20
```

- [ ] **Step 5: Commit**

```bash
git add helix-db/src/helix_engine/bm25/
git commit -m "refactor: split bm25 into lmdb_bm25 and rocks_bm25 with feature flags"
```

---

### Task 8: Refactor storage_core/mod.rs for dual backend

This is the largest single change. The LMDB implementation moves inside `pub mod lmdb { ... }` and a `pub mod rocks { ... }` module is added. A `pub use <backend>::*` re-export keeps all callers unchanged.

**Files:**
- Modify: `helix-db/src/helix_engine/storage_core/mod.rs`

- [ ] **Step 1: Fetch the full updated file from upstream**

```bash
gh api "repos/HelixDB/helix-db/contents/helix-db/src/helix_engine/storage_core/mod.rs?ref=rocks-impl" \
  --jq '.content' | base64 -d > helix-db/src/helix_engine/storage_core/mod.rs
wc -l helix-db/src/helix_engine/storage_core/mod.rs
```

Expected: ~1177 lines.

- [ ] **Step 2: Verify the file structure is correct**

```bash
grep -n "^pub mod\|^pub use\|^#\[cfg" helix-db/src/helix_engine/storage_core/mod.rs | head -30
```

Expected to see: `pub mod lmdb`, `pub mod rocks`, `pub use lmdb::*` / `pub use rocks::*`, `#[cfg(feature = "lmdb")]`, `#[cfg(feature = "rocks")]`.

- [ ] **Step 3: Check for any references to types not yet defined in our codebase**

The upstream `storage_core/mod.rs` may reference `SecondaryIndex` differently from our snapshot. In our snapshot `secondary_indices` is `HashMap<String, (Database<Bytes, U128<BE>>, SecondaryIndex)>` but in the upstream rocks-impl it is `HashMap<String, Database<Bytes, U128<BE>>>` (the enum wrapper is dropped). Check and fix:

```bash
grep "SecondaryIndex" helix-db/src/helix_engine/storage_core/mod.rs
```

If the upstream version dropped the enum wrapper, update `storage_migration.rs` and any other file referencing the tuple form.

- [ ] **Step 4: Check lmdb compilation**

```bash
cd helix-db && cargo check --features lmdb 2>&1 | grep "^error" | head -30
```

Fix any import errors — the upstream file may reference `storage_migration` inside `pub mod lmdb {}` but our `storage_migration.rs` is at the top level. Adjust the path accordingly.

- [ ] **Step 5: Commit**

```bash
git add helix-db/src/helix_engine/storage_core/mod.rs
git commit -m "feat: wrap storage_core in lmdb/rocks feature-gated modules"
```

---

### Task 9: Update traversal ops for dual backend

The traversal ops reference `HelixGraphStorage` (unchanged — still the concrete type re-exported via `pub use lmdb::*` or `pub use rocks::*`) but some also import `heed3` types directly. These need to use `RTxn`/`WTxn` instead.

**Files (fetch each from upstream):**
- Modify: `helix-db/src/helix_engine/traversal_core/ops/vectors/insert.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/ops/vectors/search.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/ops/util/paths.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/ops/util/update.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/ops/util/drop.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/ops/util/filter_ref.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/ops/util/map.rs`
- Modify: `helix-db/src/helix_engine/traversal_core/ops/source/v_from_type.rs`

- [ ] **Step 1: Fetch all updated traversal op files from upstream**

```bash
OPS="helix-db/src/helix_engine/traversal_core/ops"

for f in \
  "vectors/insert.rs" \
  "vectors/search.rs" \
  "util/paths.rs" \
  "util/update.rs" \
  "util/drop.rs" \
  "util/filter_ref.rs" \
  "util/map.rs" \
  "source/v_from_type.rs"; do
  gh api "repos/HelixDB/helix-db/contents/$OPS/$f?ref=rocks-impl" \
    --jq '.content' | base64 -d > "$OPS/$f"
  echo "wrote $OPS/$f"
done
```

- [ ] **Step 2: Remove the deleted file**

The upstream rocks-impl removes `util/filter_mut.rs`. Check if it exists and delete it:

```bash
rm -f helix-db/src/helix_engine/traversal_core/ops/util/filter_mut.rs
```

Also remove its `pub mod filter_mut;` entry from `util/mod.rs` if present:

```bash
grep -n "filter_mut" helix-db/src/helix_engine/traversal_core/ops/util/mod.rs
# If found, remove that line
```

- [ ] **Step 3: Check lmdb compilation**

```bash
cd helix-db && cargo check --features lmdb 2>&1 | grep "^error" | head -30
```

- [ ] **Step 4: Commit**

```bash
git add helix-db/src/helix_engine/traversal_core/ops/
git commit -m "feat: update traversal ops to support lmdb/rocks backends"
```

---

### Task 10: Update gateway builtins and MCP for dual backend

**Files (fetch from upstream):**
- Modify: `helix-db/src/helix_gateway/builtin/all_nodes_and_edges.rs`
- Modify: `helix-db/src/helix_gateway/builtin/node_by_id.rs`
- Modify: `helix-db/src/helix_gateway/builtin/node_connections.rs`
- Modify: `helix-db/src/helix_gateway/builtin/nodes_by_label.rs`
- Create: `helix-db/src/helix_gateway/builtin/rocks_utils.rs`
- Modify: `helix-db/src/helix_gateway/mcp/mcp.rs`
- Modify: `helix-db/src/helix_gateway/mcp/tools.rs`

- [ ] **Step 1: Fetch all gateway files from upstream**

```bash
BUILTIN="helix-db/src/helix_gateway/builtin"
MCP="helix-db/src/helix_gateway/mcp"

for f in all_nodes_and_edges.rs node_by_id.rs node_connections.rs nodes_by_label.rs rocks_utils.rs; do
  gh api "repos/HelixDB/helix-db/contents/$BUILTIN/$f?ref=rocks-impl" \
    --jq '.content' | base64 -d > "$BUILTIN/$f"
  echo "wrote $BUILTIN/$f"
done

for f in mcp.rs tools.rs; do
  gh api "repos/HelixDB/helix-db/contents/$MCP/$f?ref=rocks-impl" \
    --jq '.content' | base64 -d > "$MCP/$f"
  echo "wrote $MCP/$f"
done
```

- [ ] **Step 2: Register rocks_utils in builtin/mod.rs**

```bash
grep -n "rocks_utils\|pub mod" helix-db/src/helix_gateway/builtin/mod.rs
```

If `rocks_utils` is not already listed, add:

```rust
#[cfg(feature = "rocks")]
pub mod rocks_utils;
```

- [ ] **Step 3: Check lmdb compilation**

```bash
cd helix-db && cargo check --features lmdb 2>&1 | grep "^error" | head -30
```

- [ ] **Step 4: Commit**

```bash
git add helix-db/src/helix_gateway/
git commit -m "feat: update gateway builtins and MCP to support lmdb/rocks"
```

---

### Task 11: Update helixc generator for dual backend

**Files (fetch from upstream):**
- Modify: `helix-db/src/helixc/generator/source_steps.rs`
- Modify: `helix-db/src/helixc/generator/utils.rs`

- [ ] **Step 1: Fetch updated generator files**

```bash
GEN="helix-db/src/helixc/generator"

for f in source_steps.rs utils.rs; do
  gh api "repos/HelixDB/helix-db/contents/$GEN/$f?ref=rocks-impl" \
    --jq '.content' | base64 -d > "$GEN/$f"
  echo "wrote $GEN/$f"
done
```

- [ ] **Step 2: Check lmdb compilation**

```bash
cd helix-db && cargo check --features lmdb 2>&1 | grep "^error" | head -20
```

- [ ] **Step 3: Commit**

```bash
git add helix-db/src/helixc/generator/source_steps.rs \
        helix-db/src/helixc/generator/utils.rs
git commit -m "feat: update helixc generator for dual storage backend"
```

---

### Task 12: Final verification — both features compile

- [ ] **Step 1: Full check with lmdb feature**

```bash
cd helix-db && cargo check --features lmdb 2>&1
```

Expected: zero errors. Warnings are acceptable.

- [ ] **Step 2: Full check with rocks feature**

```bash
cd helix-db && cargo check --features rocks 2>&1
```

Expected: zero errors. Fix any remaining import or type mismatches by cross-referencing the upstream file for that specific location.

- [ ] **Step 3: Run lmdb tests**

```bash
cd helix-db && cargo test --features lmdb 2>&1 | tail -20
```

- [ ] **Step 4: Commit final fixes if any**

```bash
git add -p
git commit -m "fix: resolve remaining compilation issues for lmdb and rocks features"
```

- [ ] **Step 5: Tag the completed port**

```bash
git tag v2-rocks-port
```

---

## File Map Summary

| File | Action | Source |
|---|---|---|
| `helix-ts/ir.ts` | Create | python-typescript-dsl |
| `helix-ts/main.ts` | Create | python-typescript-dsl |
| `helix-ts/deno.json` | Create | python-typescript-dsl |
| `examples/bookstore.py` | Create | python-typescript-dsl |
| `examples/bookstore.ts` | Create | python-typescript-dsl |
| `helix-db/Cargo.toml` | Modify | manual |
| `helix-db/src/helix_engine/storage_core/txn.rs` | Create | manual (from rocks-impl txn.rs) |
| `helix-db/src/helix_engine/traversal_core/mod.rs` | Modify | append RTxn/WTxn aliases |
| `helix-db/src/helix_engine/traversal_core/traversal_iter.rs` | Modify | replace heed3 types |
| `helix-db/src/helix_engine/storage_core/storage_methods.rs` | Modify | manual (feature-flagged) |
| `helix-db/src/helix_engine/storage_core/mod.rs` | Replace | rocks-impl |
| `helix-db/src/helix_engine/vector_core/lmdb/` | Create dir | move from vector_core/ |
| `helix-db/src/helix_engine/vector_core/rocks/` | Create dir | rocks-impl |
| `helix-db/src/helix_engine/vector_core/mod.rs` | Replace | manual |
| `helix-db/src/helix_engine/bm25/lmdb_bm25.rs` | Rename from bm25.rs | — |
| `helix-db/src/helix_engine/bm25/rocks_bm25.rs` | Create | rocks-impl |
| `helix-db/src/helix_engine/bm25/mod.rs` | Replace | manual (feature-flagged) |
| `helix-db/src/helix_engine/traversal_core/ops/vectors/*.rs` | Replace | rocks-impl |
| `helix-db/src/helix_engine/traversal_core/ops/util/*.rs` | Replace | rocks-impl |
| `helix-db/src/helix_engine/traversal_core/ops/source/v_from_type.rs` | Replace | rocks-impl |
| `helix-db/src/helix_gateway/builtin/*.rs` | Replace | rocks-impl |
| `helix-db/src/helix_gateway/builtin/rocks_utils.rs` | Create | rocks-impl |
| `helix-db/src/helix_gateway/mcp/mcp.rs` | Replace | rocks-impl |
| `helix-db/src/helix_gateway/mcp/tools.rs` | Replace | rocks-impl |
| `helix-db/src/helixc/generator/source_steps.rs` | Replace | rocks-impl |
| `helix-db/src/helixc/generator/utils.rs` | Replace | rocks-impl |
