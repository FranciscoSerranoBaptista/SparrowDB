# sparrow-memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a `sparrow-memory` Rust crate that gives research agents persistent, queryable episodic memory backed by SparrowDB's raw graph storage.

**Architecture:** Schema-free library crate inside the SparrowDB workspace that opens a dedicated LMDB environment (separate from the domain graph), writes nodes and edges directly via `SparrowGraphStorage`, and exposes a `MemoryStore → ThreadHandle → RunHandle` API. No HQL compiler, no HTTP server, no traversal engine — direct storage ops only.

**Tech Stack:** Rust 2024 edition, `sparrow-db` (lmdb feature), `bumpalo`, `bincode`, `uuid` (v6), `heed3` (via sparrow-db re-exports), `tempfile` (tests)

**Spec:** `docs/superpowers/specs/2026-05-20-sparrow-memory-design.md`

---

## Key storage patterns (read before implementing)

Before writing any code, read these files:
- `sparrow-db/src/sparrow_engine/storage_core/mod.rs` — `SparrowGraphStorage` struct fields and `new()`
- `sparrow-db/src/sparrow_engine/traversal_core/ops/source/add_n.rs` — how to write a node + secondary index entries
- `sparrow-db/src/sparrow_engine/traversal_core/ops/source/add_e.rs` — how to write an edge (out/in adjacency)
- `sparrow-db/src/sparrow_engine/traversal_core/ops/source/n_from_index.rs` — how to read from a secondary index
- `sparrow-db/src/utils/properties.rs` — `ImmutablePropertiesMap::new()`
- `sparrow-db/src/utils/items.rs` — `Node`, `Edge` structs + `from_bincode_bytes`

The storage patterns that matter:

```rust
// Write a node
let node = Node { id: v6_uuid(), label, version: 1, properties: Some(props) };
let bytes = bincode::serialize(&node)?;
storage.nodes_db.put_with_flags(&mut wtxn, PutFlags::APPEND, &node.id, &bytes)?;

// Write a secondary index entry (for SecondaryIndex::Index — DUP_SORT)
let key_bytes = bincode::serialize(&Value::U128(thread_id))?;
let (idx_db, _) = storage.secondary_indices.get("finding:thread_id").unwrap();
idx_db.put(&mut wtxn, &key_bytes, &node.id)?;

// Read from a secondary index (prefix_iter returns (key_bytes, node_id) pairs)
let key_bytes = bincode::serialize(&Value::U128(thread_id))?;
let (idx_db, _) = storage.secondary_indices.get("finding:thread_id").unwrap();
for item in idx_db.prefix_iter(&rtxn, &key_bytes)? {
    let (_, node_id) = item?;
    let arena = bumpalo::Bump::new();
    let node = storage.get_node(&rtxn, node_id, &arena)?;
    // read properties from node
}

// Write an edge (out + in adjacency)
let edge_id = v6_uuid();
let label_hash = hash_label(edge_label, None);
let out_key = SparrowGraphStorage::out_edge_key(&from_id, &label_hash);
let in_key  = SparrowGraphStorage::in_edge_key(&to_id, &label_hash);
let packed_out = SparrowGraphStorage::pack_edge_data(&edge_id, &to_id);
let packed_in  = SparrowGraphStorage::pack_edge_data(&edge_id, &from_id);
storage.out_edges_db.put(&mut wtxn, &out_key[..], &packed_out[..])?;
storage.in_edges_db.put(&mut wtxn, &in_key[..], &packed_in[..])?;
```

---

## File map

| File | Responsibility |
|---|---|
| `sparrow-memory/Cargo.toml` | Crate manifest, workspace membership |
| `sparrow-memory/src/lib.rs` | Public re-exports |
| `sparrow-memory/src/error.rs` | `MemoryError` wrapping `GraphError` |
| `sparrow-memory/src/types.rs` | `Finding`, `StoredFinding`, `StoredSummary`, `StoredQuestion`, `Priority`, `FindingId`, `QuestionId`, `ThreadSummary`, `RecallResult` |
| `sparrow-memory/src/indices.rs` | Secondary index name constants |
| `sparrow-memory/src/graph.rs` | Low-level node/edge write + read helpers |
| `sparrow-memory/src/store.rs` | `MemoryStore`, `MemoryConfig`, `open()` |
| `sparrow-memory/src/thread.rs` | `ThreadHandle`, `recall()`, `count_distinct()`, `findings_for_entity()` |
| `sparrow-memory/src/run.rs` | `RunHandle`, `record_finding()`, `raise_question()`, `answer_question()`, `complete()`, `interrupt()` |
| `sparrow-memory/src/recall.rs` | `RecallResult` assembly from secondary index scans |
| `sparrow-memory/tests/integration.rs` | End-to-end tests with temp dir |

---

## Task 1: Crate scaffold

**Files:**
- Create: `sparrow-memory/Cargo.toml`
- Create: `sparrow-memory/src/lib.rs`
- Create: `sparrow-memory/src/error.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Create `sparrow-memory/Cargo.toml`**

```toml
[package]
name = "sparrow-memory"
version = "0.1.0"
edition = "2024"
description = "Lightweight episodic memory for research agents, backed by SparrowDB."

[dependencies]
sparrow-db = { path = "../sparrow-db", default-features = false, features = ["lmdb", "vectors"] }
bumpalo = { version = "3.19.0", features = ["collections", "boxed"] }
bincode = "1.3.3"
uuid = { version = "1.12.1", features = ["v4", "v6", "fast-rng"] }
heed3 = { version = "0.22.0" }
thiserror = "2.0.12"
serde = { version = "1.0.217", features = ["derive"] }

[dev-dependencies]
tempfile = "3.20.0"
```

- [ ] **Step 2: Add crate to workspace `Cargo.toml`**

Open `/Cargo.toml`. In the `[workspace]` `members` list, add `"sparrow-memory"`.

```toml
members = [
    "sparrow-db",
    "sparrow-cli",
    "sparrow-container",
    "sparrow-macros",
    "metrics",
    "hql-tests",
    "sparrow-memory",   # ← add this
]
```

- [ ] **Step 3: Create `sparrow-memory/src/error.rs`**

```rust
use sparrow_db::sparrow_engine::types::GraphError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("storage error: {0}")]
    Storage(#[from] GraphError),
    #[error("serialization error: {0}")]
    Serialization(#[from] bincode::Error),
    #[error("index not found: {0}")]
    IndexNotFound(String),
    #[error("node not found: {0}")]
    NodeNotFound(u128),
    #[error("heed error: {0}")]
    Heed(#[from] heed3::Error),
}
```

- [ ] **Step 4: Create `sparrow-memory/src/lib.rs`**

```rust
pub mod error;
pub mod graph;
pub mod indices;
pub mod recall;
pub mod run;
pub mod store;
pub mod thread;
pub mod types;

pub use error::MemoryError;
pub use run::RunHandle;
pub use store::{MemoryConfig, MemoryStore};
pub use thread::ThreadHandle;
pub use types::{Finding, Priority, RecallResult};
```

- [ ] **Step 5: Verify it compiles**

```bash
cargo build -p sparrow-memory
```

Expected: compiles (no src files yet beyond error.rs/lib.rs so expect missing module errors — that's fine, confirm the workspace linking works).

- [ ] **Step 6: Commit**

```bash
git add sparrow-memory/ Cargo.toml Cargo.lock
git commit -m "chore(memory): scaffold sparrow-memory crate"
```

---

## Task 2: Types

**Files:**
- Create: `sparrow-memory/src/types.rs`
- Create: `sparrow-memory/src/indices.rs`

- [ ] **Step 1: Create `sparrow-memory/src/indices.rs`**

These string constants are the secondary index names registered in LMDB and used in every scan.

```rust
/// Secondary index for finding nodes → look up by thread_id
pub const FINDING_THREAD_ID: &str = "finding:thread_id";
/// Secondary index for finding nodes → look up by entity_id (for count_distinct)
pub const FINDING_ENTITY_ID: &str = "finding:entity_id";
/// Secondary index for open_question nodes → look up by thread_id
pub const QUESTION_THREAD_ID: &str = "question:thread_id";
/// Secondary index for agent_run nodes → look up by thread_id
pub const RUN_THREAD_ID: &str = "run:thread_id";
/// Secondary index for run_summary nodes → look up by thread_id
pub const SUMMARY_THREAD_ID: &str = "summary:thread_id";

/// All index names — passed to Config on MemoryStore::open
pub const ALL_INDICES: &[&str] = &[
    FINDING_THREAD_ID,
    FINDING_ENTITY_ID,
    QUESTION_THREAD_ID,
    RUN_THREAD_ID,
    SUMMARY_THREAD_ID,
];
```

- [ ] **Step 2: Create `sparrow-memory/src/types.rs`**

```rust
use sparrow_db::protocol::value::Value;
use std::collections::HashMap;

// ── Opaque ID newtypes ────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FindingId(pub u128);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuestionId(pub u128);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadId(pub u128);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunId(pub u128);

// ── Input types (what the agent provides) ────────────────────────────

#[derive(Debug, Clone)]
pub struct Finding {
    /// The finding itself — what the agent concluded. Indexed for recall.
    pub claim: String,
    /// 0.0–1.0
    pub confidence: f32,
    /// Opaque foreign reference into the domain graph (e.g. a sacred_cow node ID).
    pub entity_id: Option<u128>,
    /// Label of the referenced entity, e.g. "sacred_cow", "intervention".
    pub entity_label: Option<String>,
    /// Agent-specific metadata. Stored as Value::Object. Library never inspects it.
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    High,
    Medium,
    Low,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
        }
    }
}

// ── Stored types (what recall returns) ───────────────────────────────

#[derive(Debug, Clone)]
pub struct StoredFinding {
    pub id: FindingId,
    pub claim: String,
    pub confidence: f32,
    pub entity_id: Option<u128>,
    pub entity_label: Option<String>,
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct StoredSummary {
    pub run_id: RunId,
    pub summary: String,
    pub finding_count: u32,
    pub question_count: u32,
}

#[derive(Debug, Clone)]
pub struct StoredQuestion {
    pub id: QuestionId,
    pub question: String,
    pub priority: String,
}

#[derive(Debug, Clone)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub name: String,
    pub goal: String,
    pub status: String,
}

// ── Recall result ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct RecallResult {
    /// Summaries of the last 3 completed runs, newest first.
    pub recent_summaries: Vec<StoredSummary>,
    /// Top-K findings from the thread, ordered by recency.
    pub relevant_findings: Vec<StoredFinding>,
    /// All open questions for this thread.
    pub open_questions: Vec<StoredQuestion>,
}
```

- [ ] **Step 3: Verify types compile**

```bash
cargo build -p sparrow-memory 2>&1 | grep "error\[" | head -20
```

Expected: errors only for missing modules (graph, store, thread, run, recall) — not for types.rs or indices.rs.

- [ ] **Step 4: Commit**

```bash
git add sparrow-memory/src/types.rs sparrow-memory/src/indices.rs
git commit -m "feat(memory): add types and index name constants"
```

---

## Task 3: Graph primitives

**Files:**
- Create: `sparrow-memory/src/graph.rs`

This module is the only place that touches `SparrowGraphStorage` directly. All other modules go through here.

- [ ] **Step 1: Write a failing test**

In `sparrow-memory/tests/integration.rs`:

```rust
use sparrow_memory::graph::{write_node, NodeProps};
use sparrow_db::{
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, StorageConfig, version_info::VersionInfo},
        traversal_core::config::Config,
    },
    protocol::value::Value,
};
use tempfile::TempDir;

fn open_test_storage() -> (SparrowGraphStorage, TempDir) {
    let dir = TempDir::new().unwrap();
    let config = Config::default();
    let vi = VersionInfo::default();
    let storage = SparrowGraphStorage::new(dir.path().to_str().unwrap(), config, vi).unwrap();
    (storage, dir)
}

#[test]
fn test_write_and_read_node() {
    let (storage, _dir) = open_test_storage();
    let props = vec![
        ("claim", Value::String("test finding".to_string())),
        ("confidence", Value::F32(0.9)),
    ];
    let id = write_node(&storage, "finding", props).unwrap();
    let arena = bumpalo::Bump::new();
    let rtxn = storage.graph_env.read_txn().unwrap();
    let node = storage.get_node(&rtxn, id, &arena).unwrap();
    assert_eq!(node.label, "finding");
    assert_eq!(
        node.get_property("claim"),
        Some(&Value::String("test finding".to_string()))
    );
}
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cargo test -p sparrow-memory test_write_and_read_node 2>&1 | tail -5
```

Expected: compile error — `graph` module doesn't exist yet.

- [ ] **Step 3: Create `sparrow-memory/src/graph.rs`**

```rust
use bumpalo::Bump;
use sparrow_db::{
    protocol::value::Value,
    sparrow_engine::{
        storage_core::SparrowGraphStorage,
        storage_core::storage_methods::StorageMethods,
        types::GraphError,
    },
    utils::{
        id::v6_uuid,
        items::{Edge, Node},
        label_hash::hash_label,
        properties::ImmutablePropertiesMap,
    },
};
use bincode::Options;
use heed3::PutFlags;

use crate::error::MemoryError;

/// A list of (property_name, Value) pairs to store on a node.
pub type NodeProps<'a> = Vec<(&'a str, Value)>;

/// Write a new node to storage and return its ID.
/// Properties are built using a short-lived arena — this fn owns the full write transaction.
pub fn write_node(
    storage: &SparrowGraphStorage,
    label: &str,
    props: NodeProps<'_>,
) -> Result<u128, MemoryError> {
    let arena = Bump::new();
    let label = arena.alloc_str(label);
    let len = props.len();
    let arena_props: Vec<(&str, Value)> = props
        .into_iter()
        .map(|(k, v)| (arena.alloc_str(k) as &str, v))
        .collect();

    let properties = if len == 0 {
        None
    } else {
        Some(ImmutablePropertiesMap::new(
            len,
            arena_props.into_iter(),
            &arena,
        ))
    };

    let id = v6_uuid();
    let node = Node {
        id,
        label,
        version: 1,
        properties,
    };

    let bytes = bincode::serialize(&node).map_err(MemoryError::Serialization)?;

    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;
    storage
        .nodes_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &id, &bytes)
        .map_err(MemoryError::Heed)?;
    wtxn.commit().map_err(MemoryError::Heed)?;

    Ok(id)
}

/// Write a new node AND register it in one or more secondary indices.
/// `index_entries`: list of (index_name, Value to index by).
pub fn write_node_indexed(
    storage: &SparrowGraphStorage,
    label: &str,
    props: NodeProps<'_>,
    index_entries: &[(&str, Value)],
) -> Result<u128, MemoryError> {
    let arena = Bump::new();
    let label = arena.alloc_str(label);
    let len = props.len();
    let arena_props: Vec<(&str, Value)> = props
        .into_iter()
        .map(|(k, v)| (arena.alloc_str(k) as &str, v))
        .collect();

    let properties = if len == 0 {
        None
    } else {
        Some(ImmutablePropertiesMap::new(
            len,
            arena_props.into_iter(),
            &arena,
        ))
    };

    let id = v6_uuid();
    let node = Node {
        id,
        label,
        version: 1,
        properties,
    };

    let bytes = bincode::serialize(&node).map_err(MemoryError::Serialization)?;

    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;

    storage
        .nodes_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &id, &bytes)
        .map_err(MemoryError::Heed)?;

    for (index_name, index_value) in index_entries {
        let (idx_db, _) = storage
            .secondary_indices
            .get(*index_name)
            .ok_or_else(|| MemoryError::IndexNotFound(index_name.to_string()))?;
        let key_bytes =
            bincode::serialize(index_value).map_err(MemoryError::Serialization)?;
        idx_db
            .put(&mut wtxn, &key_bytes, &id)
            .map_err(MemoryError::Heed)?;
    }

    wtxn.commit().map_err(MemoryError::Heed)?;

    Ok(id)
}

/// Read all node IDs from a secondary index that match a given Value key.
pub fn ids_from_index(
    storage: &SparrowGraphStorage,
    index_name: &str,
    key: &Value,
) -> Result<Vec<u128>, MemoryError> {
    let (idx_db, _) = storage
        .secondary_indices
        .get(index_name)
        .ok_or_else(|| MemoryError::IndexNotFound(index_name.to_string()))?;

    let key_bytes = bincode::serialize(key).map_err(MemoryError::Serialization)?;
    let rtxn = storage.graph_env.read_txn().map_err(MemoryError::Heed)?;

    let mut ids = Vec::new();
    for item in idx_db
        .prefix_iter(&rtxn, &key_bytes)
        .map_err(MemoryError::Heed)?
    {
        let (_, node_id) = item.map_err(MemoryError::Heed)?;
        ids.push(node_id);
    }
    Ok(ids)
}

/// Read a node by its ID, deserializing into owned Strings for properties.
/// Returns (label, properties as HashMap) for easy access outside the arena.
pub fn read_node_props(
    storage: &SparrowGraphStorage,
    id: u128,
) -> Result<(String, std::collections::HashMap<String, Value>), MemoryError> {
    let arena = Bump::new();
    let rtxn = storage.graph_env.read_txn().map_err(MemoryError::Heed)?;
    let node = storage
        .get_node(&rtxn, id, &arena)
        .map_err(MemoryError::Storage)?;

    let label = node.label.to_owned();
    let mut map = std::collections::HashMap::new();
    if let Some(props) = node.properties {
        for (k, v) in props.iter() {
            map.insert(k.to_owned(), v.clone());
        }
    }
    Ok((label, map))
}

/// Write a directed edge between two nodes (no properties).
pub fn write_edge(
    storage: &SparrowGraphStorage,
    label: &str,
    from_id: u128,
    to_id: u128,
) -> Result<u128, MemoryError> {
    let arena = Bump::new();
    let label_ref = arena.alloc_str(label);
    let label_hash = hash_label(label_ref, None);

    let edge_id = v6_uuid();
    let edge = Edge {
        id: edge_id,
        label: label_ref,
        version: 1,
        from_id,
        to_id,
        properties: None,
    };

    let bytes = bincode::serialize(&edge).map_err(MemoryError::Serialization)?;
    let out_key = SparrowGraphStorage::out_edge_key(&from_id, &label_hash);
    let in_key = SparrowGraphStorage::in_edge_key(&to_id, &label_hash);
    let packed_out = SparrowGraphStorage::pack_edge_data(&edge_id, &to_id);
    let packed_in = SparrowGraphStorage::pack_edge_data(&edge_id, &from_id);

    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;

    storage
        .edges_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &edge_id, &bytes)
        .map_err(MemoryError::Heed)?;

    storage
        .out_edges_db
        .put(&mut wtxn, &out_key[..], &packed_out[..])
        .map_err(MemoryError::Heed)?;

    storage
        .in_edges_db
        .put(&mut wtxn, &in_key[..], &packed_in[..])
        .map_err(MemoryError::Heed)?;

    wtxn.commit().map_err(MemoryError::Heed)?;

    Ok(edge_id)
}

/// Get the IDs of all nodes reachable via an out-edge of a given label from `from_id`.
pub fn out_neighbors(
    storage: &SparrowGraphStorage,
    from_id: u128,
    edge_label: &str,
) -> Result<Vec<u128>, MemoryError> {
    let label_hash = hash_label(edge_label, None);
    let prefix = SparrowGraphStorage::out_edge_key(&from_id, &label_hash);
    let rtxn = storage.graph_env.read_txn().map_err(MemoryError::Heed)?;

    let mut neighbors = Vec::new();
    if let Some(iter) = storage
        .out_edges_db
        .get_duplicates(&rtxn, &prefix[..])
        .map_err(MemoryError::Heed)?
    {
        for item in iter {
            let (_, packed) = item.map_err(MemoryError::Heed)?;
            // packed = [edge_id: 16 bytes][to_node_id: 16 bytes]
            if packed.len() >= 32 {
                let to_id = u128::from_be_bytes(packed[16..32].try_into().unwrap());
                neighbors.push(to_id);
            }
        }
    }
    Ok(neighbors)
}
```

- [ ] **Step 4: Run test to confirm it passes**

```bash
cargo test -p sparrow-memory test_write_and_read_node -- --nocapture
```

Expected: `test test_write_and_read_node ... ok`

- [ ] **Step 5: Commit**

```bash
git add sparrow-memory/src/graph.rs sparrow-memory/tests/integration.rs
git commit -m "feat(memory): add graph primitive helpers (write_node, write_edge, index scan)"
```

---

## Task 4: MemoryStore — open with secondary indices

**Files:**
- Create: `sparrow-memory/src/store.rs`

- [ ] **Step 1: Write failing test**

Add to `sparrow-memory/tests/integration.rs`:

```rust
use sparrow_memory::{MemoryConfig, MemoryStore};

#[test]
fn test_memory_store_opens() {
    let dir = TempDir::new().unwrap();
    let cfg = MemoryConfig {
        path: dir.path().to_str().unwrap().to_string(),
        db_max_size_gb: Some(1),
    };
    let store = MemoryStore::open(cfg).unwrap();
    // confirm all 5 indices are registered
    assert_eq!(store.index_names().len(), 5);
}
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cargo test -p sparrow-memory test_memory_store_opens 2>&1 | tail -5
```

Expected: compile error — `store` module not found.

- [ ] **Step 3: Create `sparrow-memory/src/store.rs`**

```rust
use sparrow_db::{
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, version_info::VersionInfo},
        traversal_core::config::{Config, GraphConfig},
        types::SecondaryIndex,
    },
};
use std::sync::Arc;

use crate::{error::MemoryError, indices::ALL_INDICES, thread::ThreadHandle};

pub struct MemoryConfig {
    pub path: String,
    pub db_max_size_gb: Option<usize>,
}

pub struct MemoryStore {
    pub(crate) storage: Arc<SparrowGraphStorage>,
}

impl MemoryStore {
    pub fn open(config: MemoryConfig) -> Result<Self, MemoryError> {
        std::fs::create_dir_all(&config.path)
            .map_err(|e| MemoryError::Storage(sparrow_db::sparrow_engine::types::GraphError::Io(e)))?;

        let secondary_indices: Vec<SecondaryIndex> = ALL_INDICES
            .iter()
            .map(|name| SecondaryIndex::Index(name.to_string()))
            .collect();

        let sparrow_config = Config {
            vector_config: None,
            graph_config: Some(GraphConfig {
                secondary_indices: Some(secondary_indices),
            }),
            db_max_size_gb: config.db_max_size_gb,
            mcp: Some(false),
            bm25: Some(false),
            schema: None,
            embedding_model: None,
            graphvis_node_label: None,
            hql_schema_raw: None,
        };

        let storage = SparrowGraphStorage::new(
            &config.path,
            sparrow_config,
            VersionInfo::default(),
        )
        .map_err(MemoryError::Storage)?;

        Ok(Self {
            storage: Arc::new(storage),
        })
    }

    /// List all registered secondary index names (useful for tests).
    pub fn index_names(&self) -> Vec<String> {
        self.storage.secondary_indices.keys().cloned().collect()
    }

    /// Get or create a research thread for `agent` with the given `name`.
    /// `goal` is only used when creating — ignored on subsequent calls.
    pub fn thread(
        &self,
        agent: &str,
        name: &str,
        goal: &str,
    ) -> Result<ThreadHandle, MemoryError> {
        ThreadHandle::get_or_create(Arc::clone(&self.storage), agent, name, goal)
    }
}
```

- [ ] **Step 4: Fix the `GraphError::Io` reference** — check `sparrow-db/src/sparrow_engine/types.rs` for how IO errors are wrapped:

```bash
grep -n "Io\|io::" /Users/franciscobaptista/Development/SparrowDB/sparrow-db/src/sparrow_engine/types.rs | head -10
```

If `GraphError` doesn't have an `Io` variant, replace with `GraphError::New(e.to_string())`.

- [ ] **Step 5: Run test**

```bash
cargo test -p sparrow-memory test_memory_store_opens -- --nocapture
```

Expected: `ok`

- [ ] **Step 6: Commit**

```bash
git add sparrow-memory/src/store.rs
git commit -m "feat(memory): add MemoryStore::open with secondary index registration"
```

---

## Task 5: ThreadHandle — get or create

**Files:**
- Create: `sparrow-memory/src/thread.rs`

Node label for research threads: `"research_thread"`
Properties stored: `name`, `goal`, `agent_name`, `status` (Value::String)

- [ ] **Step 1: Write failing test**

Add to `sparrow-memory/tests/integration.rs`:

```rust
#[test]
fn test_thread_idempotent() {
    let dir = TempDir::new().unwrap();
    let store = MemoryStore::open(MemoryConfig {
        path: dir.path().to_str().unwrap().to_string(),
        db_max_size_gb: Some(1),
    }).unwrap();

    let t1 = store.thread("Eye1", "sacred-cow-research", "Which interventions work for which sacred cows?").unwrap();
    let t2 = store.thread("Eye1", "sacred-cow-research", "ignored goal").unwrap();

    // same thread_id returned both times
    assert_eq!(t1.thread_id(), t2.thread_id());
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test -p sparrow-memory test_thread_idempotent 2>&1 | tail -5
```

Expected: compile error — `thread` module not found.

- [ ] **Step 3: Create `sparrow-memory/src/thread.rs`**

The thread lookup uses a full scan of `research_thread` nodes filtered by `agent_name` + `name` (no secondary index needed — there won't be thousands of threads).

```rust
use std::sync::Arc;
use std::collections::HashMap;
use sparrow_db::{protocol::value::Value, sparrow_engine::storage_core::SparrowGraphStorage};

use crate::{
    error::MemoryError,
    graph::{ids_from_index, out_neighbors, read_node_props, write_edge, write_node},
    indices::{RUN_THREAD_ID, SUMMARY_THREAD_ID},
    run::RunHandle,
    types::{
        FindingId, QuestionId, RecallResult, RunId, StoredFinding, StoredQuestion,
        StoredSummary, ThreadId,
    },
};

pub struct ThreadHandle {
    pub(crate) storage: Arc<SparrowGraphStorage>,
    pub(crate) id: u128,
}

impl ThreadHandle {
    pub fn thread_id(&self) -> u128 {
        self.id
    }

    /// Get an existing thread (matched by agent_name + name) or create one.
    pub fn get_or_create(
        storage: Arc<SparrowGraphStorage>,
        agent: &str,
        name: &str,
        goal: &str,
    ) -> Result<Self, MemoryError> {
        // Full scan of research_thread nodes to find a match.
        // Thread counts are small (dozens per agent), so this is acceptable.
        let rtxn = storage.graph_env.read_txn().map_err(MemoryError::Heed)?;
        let arena = bumpalo::Bump::new();

        // Iterate all nodes_db entries, filter by label == "research_thread"
        for item in storage.nodes_db.iter(&rtxn).map_err(MemoryError::Heed)? {
            let (node_id, _) = item.map_err(MemoryError::Heed)?;
            let node = match storage.get_node(&rtxn, node_id, &arena) {
                Ok(n) => n,
                Err(_) => continue,
            };
            if node.label != "research_thread" {
                continue;
            }
            let agent_match = node
                .get_property("agent_name")
                .map(|v| matches!(v, Value::String(s) if s == agent))
                .unwrap_or(false);
            let name_match = node
                .get_property("name")
                .map(|v| matches!(v, Value::String(s) if s == name))
                .unwrap_or(false);
            if agent_match && name_match {
                return Ok(Self { storage, id: node_id });
            }
        }
        drop(rtxn);

        // Not found — create it.
        let id = write_node(
            &storage,
            "research_thread",
            vec![
                ("name", Value::String(name.to_string())),
                ("goal", Value::String(goal.to_string())),
                ("agent_name", Value::String(agent.to_string())),
                ("status", Value::String("active".to_string())),
            ],
        )?;

        Ok(Self { storage, id })
    }

    /// Start a new run against this thread.
    /// Creates an `agent_run` node and chains a FOLLOWS edge from the last run.
    pub fn start_run(&self) -> Result<RunHandle, MemoryError> {
        RunHandle::create(Arc::clone(&self.storage), self.id)
    }

    /// Retrieve context for a Claude call: recent summaries + recent findings + open questions.
    pub fn recall(&self, _query: &str) -> Result<RecallResult, MemoryError> {
        crate::recall::build_recall(Arc::clone(&self.storage), self.id)
    }

    /// Count distinct entity_ids of a given label across all findings in this thread.
    /// Findings where entity_id is None are excluded.
    pub fn count_distinct(&self, entity_label: &str) -> Result<usize, MemoryError> {
        let finding_ids =
            ids_from_index(&self.storage, FINDING_THREAD_ID, &Value::U128(self.id))?;

        let mut distinct = std::collections::HashSet::new();
        for fid in finding_ids {
            let (label, props) = read_node_props(&self.storage, fid)?;
            if label != "finding" {
                continue;
            }
            // Filter by entity_label
            let el_match = props
                .get("entity_label")
                .map(|v| matches!(v, Value::String(s) if s == entity_label))
                .unwrap_or(false);
            if !el_match {
                continue;
            }
            if let Some(Value::U128(eid)) = props.get("entity_id") {
                distinct.insert(*eid);
            }
        }
        Ok(distinct.len())
    }

    /// All findings that reference a specific domain entity.
    pub fn findings_for_entity(&self, entity_id: u128) -> Result<Vec<StoredFinding>, MemoryError> {
        let all_ids = ids_from_index(
            &self.storage,
            crate::indices::FINDING_ENTITY_ID,
            &Value::U128(entity_id),
        )?;

        let mut findings = Vec::new();
        for fid in all_ids {
            let (label, props) = read_node_props(&self.storage, fid)?;
            if label != "finding" {
                continue;
            }
            // Only include findings belonging to this thread
            let in_thread = props
                .get("thread_id")
                .map(|v| matches!(v, Value::U128(tid) if *tid == self.id))
                .unwrap_or(false);
            if !in_thread {
                continue;
            }
            findings.push(props_to_stored_finding(fid, props));
        }
        Ok(findings)
    }
}

pub(crate) fn props_to_stored_finding(
    id: u128,
    props: HashMap<String, Value>,
) -> StoredFinding {
    StoredFinding {
        id: FindingId(id),
        claim: props.get("claim")
            .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
            .unwrap_or_default(),
        confidence: props.get("confidence")
            .and_then(|v| if let Value::F32(f) = v { Some(*f) } else { None })
            .unwrap_or(0.0),
        entity_id: props.get("entity_id")
            .and_then(|v| if let Value::U128(id) = v { Some(*id) } else { None }),
        entity_label: props.get("entity_label")
            .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None }),
        metadata: props.get("metadata")
            .and_then(|v| if let Value::Object(m) = v { Some(m.clone()) } else { None })
            .unwrap_or_default(),
    }
}
```

- [ ] **Step 4: Run test**

```bash
cargo test -p sparrow-memory test_thread_idempotent -- --nocapture
```

Expected: `ok`

- [ ] **Step 5: Commit**

```bash
git add sparrow-memory/src/thread.rs
git commit -m "feat(memory): add ThreadHandle with get_or_create, count_distinct, findings_for_entity"
```

---

## Task 6: RunHandle — write operations

**Files:**
- Create: `sparrow-memory/src/run.rs`

Node labels: `"agent_run"`, `"finding"`, `"open_question"`, `"run_summary"`
Edge labels: `"HAS_RUN"`, `"FOLLOWS"`, `"PRODUCED"`, `"RAISED"`, `"CARRIED"`, `"ANSWERS"`, `"SUMMARIZED_AS"`

- [ ] **Step 1: Write failing tests**

Add to `sparrow-memory/tests/integration.rs`:

```rust
#[test]
fn test_run_record_finding_and_complete() {
    use sparrow_memory::types::{Finding, Priority};
    use std::collections::HashMap;

    let dir = TempDir::new().unwrap();
    let store = MemoryStore::open(MemoryConfig {
        path: dir.path().to_str().unwrap().to_string(),
        db_max_size_gb: Some(1),
    }).unwrap();

    let thread = store.thread("Eye1", "test-thread", "test goal").unwrap();
    let run = thread.start_run().unwrap();

    let fid = run.record_finding(Finding {
        claim: "Confrontational reframing backfires in hierarchical cultures".to_string(),
        confidence: 0.82,
        entity_id: Some(999u128),
        entity_label: Some("sacred_cow".to_string()),
        metadata: HashMap::new(),
    }).unwrap();

    let qid = run.raise_question("Does industry moderate effectiveness?", Priority::High).unwrap();

    run.answer_question(qid, fid).unwrap();

    run.complete("Identified authority-culture moderation effect.").unwrap();

    // Recall should now show the summary
    let ctx = thread.recall("").unwrap();
    assert_eq!(ctx.recent_summaries.len(), 1);
    assert_eq!(ctx.relevant_findings.len(), 1);
    assert!(ctx.open_questions.is_empty()); // answered question is resolved
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test -p sparrow-memory test_run_record_finding_and_complete 2>&1 | tail -5
```

Expected: compile error — `run` module not found.

- [ ] **Step 3: Create `sparrow-memory/src/run.rs`**

```rust
use std::sync::Arc;
use sparrow_db::{protocol::value::Value, sparrow_engine::storage_core::SparrowGraphStorage};

use crate::{
    error::MemoryError,
    graph::{ids_from_index, out_neighbors, read_node_props, write_edge, write_node, write_node_indexed},
    indices::{FINDING_ENTITY_ID, FINDING_THREAD_ID, QUESTION_THREAD_ID, RUN_THREAD_ID, SUMMARY_THREAD_ID},
    types::{Finding, FindingId, Priority, QuestionId, RunId},
};

pub struct RunHandle {
    pub(crate) storage: Arc<SparrowGraphStorage>,
    pub(crate) run_id: u128,
    pub(crate) thread_id: u128,
}

impl RunHandle {
    /// Create a new agent_run node, chain FOLLOWS from last run, and write HAS_RUN from thread.
    pub(crate) fn create(
        storage: Arc<SparrowGraphStorage>,
        thread_id: u128,
    ) -> Result<Self, MemoryError> {
        // Create agent_run node
        let run_id = write_node_indexed(
            &storage,
            "agent_run",
            vec![
                ("thread_id", Value::U128(thread_id)),
                ("status", Value::String("running".to_string())),
            ],
            &[(RUN_THREAD_ID, Value::U128(thread_id))],
        )?;

        // HAS_RUN edge: thread → run
        write_edge(&storage, "HAS_RUN", thread_id, run_id)?;

        // Find previous run and write FOLLOWS edge: new_run → prev_run
        let prev_runs = ids_from_index(&storage, RUN_THREAD_ID, &Value::U128(thread_id))?;
        // prev_runs includes the one we just created; find the most recent other one
        let prev = prev_runs.iter().filter(|&&id| id != run_id).max().copied();
        if let Some(prev_run_id) = prev {
            write_edge(&storage, "FOLLOWS", run_id, prev_run_id)?;
        }

        // Carry all open questions from previous run forward
        if let Some(prev_run_id) = prev {
            let open_q_ids = ids_from_index(&storage, QUESTION_THREAD_ID, &Value::U128(thread_id))?;
            for qid in open_q_ids {
                let (_, props) = read_node_props(&storage, qid)?;
                let is_open = props
                    .get("status")
                    .map(|v| matches!(v, Value::String(s) if s == "open"))
                    .unwrap_or(false);
                if is_open {
                    write_edge(&storage, "CARRIED", run_id, qid)?;
                }
            }
        }

        Ok(Self { storage, run_id, thread_id })
    }

    /// Record a finding produced in this run.
    pub fn record_finding(&self, finding: Finding) -> Result<FindingId, MemoryError> {
        let mut props = vec![
            ("claim", Value::String(finding.claim)),
            ("confidence", Value::F32(finding.confidence)),
            ("run_id", Value::U128(self.run_id)),
            ("thread_id", Value::U128(self.thread_id)),
        ];
        if let Some(eid) = finding.entity_id {
            props.push(("entity_id", Value::U128(eid)));
        }
        if let Some(el) = finding.entity_label {
            props.push(("entity_label", Value::String(el)));
        }
        if !finding.metadata.is_empty() {
            props.push(("metadata", Value::Object(finding.metadata)));
        }

        let mut index_entries = vec![(FINDING_THREAD_ID, Value::U128(self.thread_id))];
        if let Some(eid) = finding.entity_id {
            // We stored entity_id in props above but need it for index_entries too
            // Re-derive it from props — entity_id is always present when provided
            index_entries.push((FINDING_ENTITY_ID, Value::U128(eid)));
        }

        let id = write_node_indexed(&self.storage, "finding", props, &index_entries)?;
        write_edge(&self.storage, "PRODUCED", self.run_id, id)?;

        Ok(FindingId(id))
    }

    /// Raise an open question to be resolved in this or a future run.
    pub fn raise_question(
        &self,
        question: &str,
        priority: Priority,
    ) -> Result<QuestionId, MemoryError> {
        let id = write_node_indexed(
            &self.storage,
            "open_question",
            vec![
                ("question", Value::String(question.to_string())),
                ("priority", Value::String(priority.as_str().to_string())),
                ("status", Value::String("open".to_string())),
                ("thread_id", Value::U128(self.thread_id)),
                ("run_id", Value::U128(self.run_id)),
            ],
            &[(QUESTION_THREAD_ID, Value::U128(self.thread_id))],
        )?;
        write_edge(&self.storage, "RAISED", self.run_id, id)?;
        Ok(QuestionId(id))
    }

    /// Mark a question as resolved by a finding. Writes an ANSWERS edge and flips status.
    pub fn answer_question(
        &self,
        QuestionId(qid): QuestionId,
        FindingId(fid): FindingId,
    ) -> Result<(), MemoryError> {
        // Write ANSWERS edge: finding → open_question
        write_edge(&self.storage, "ANSWERS", fid, qid)?;

        // Update status on the question node by writing a new node with the same ID.
        // Sparrow-db put() overwrites on same key, so we overwrite with updated props.
        let (_, mut props) = read_node_props(&self.storage, qid)?;
        props.insert("status".to_string(), Value::String("resolved".to_string()));
        let props_vec: Vec<(&str, Value)> = props
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();

        // Re-write the node (same ID) with updated status
        let arena = bumpalo::Bump::new();
        let label = arena.alloc_str("open_question");
        let len = props_vec.len();
        let arena_props: Vec<(&str, Value)> = props_vec
            .into_iter()
            .map(|(k, v)| (arena.alloc_str(k) as &str, v))
            .collect();
        let properties = sparrow_db::utils::properties::ImmutablePropertiesMap::new(
            len,
            arena_props.into_iter(),
            &arena,
        );
        let node = sparrow_db::utils::items::Node {
            id: qid,
            label,
            version: 2,
            properties: Some(properties),
        };
        let bytes = bincode::serialize(&node).map_err(MemoryError::Serialization)?;
        let mut wtxn = self.storage.graph_env.write_txn().map_err(MemoryError::Heed)?;
        self.storage
            .nodes_db
            .put(&mut wtxn, &qid, &bytes)
            .map_err(MemoryError::Heed)?;
        wtxn.commit().map_err(MemoryError::Heed)?;

        Ok(())
    }

    /// Mark run complete: write run_summary node, update run status.
    pub fn complete(self, summary: &str) -> Result<(), MemoryError> {
        // Count findings and open questions produced in this run
        let finding_ids = out_neighbors(&self.storage, self.run_id, "PRODUCED")?;
        let question_ids = out_neighbors(&self.storage, self.run_id, "RAISED")?;
        let finding_count = finding_ids.len() as u32;

        // Count questions that are still open
        let open_count = question_ids
            .iter()
            .filter(|&&qid| {
                read_node_props(&self.storage, qid)
                    .map(|(_, props)| {
                        props.get("status")
                            .map(|v| matches!(v, Value::String(s) if s == "open"))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            })
            .count() as u32;

        // Write run_summary node
        let summary_id = write_node_indexed(
            &self.storage,
            "run_summary",
            vec![
                ("summary", Value::String(summary.to_string())),
                ("finding_count", Value::U32(finding_count)),
                ("question_count", Value::U32(open_count)),
                ("run_id", Value::U128(self.run_id)),
                ("thread_id", Value::U128(self.thread_id)),
            ],
            &[(SUMMARY_THREAD_ID, Value::U128(self.thread_id))],
        )?;
        write_edge(&self.storage, "SUMMARIZED_AS", self.run_id, summary_id)?;

        // Update agent_run status to "completed"
        self.update_run_status("completed")?;

        Ok(())
    }

    /// Mark run interrupted — open questions are carried forward on next start_run.
    pub fn interrupt(self) -> Result<(), MemoryError> {
        self.update_run_status("interrupted")
    }

    fn update_run_status(self, status: &str) -> Result<(), MemoryError> {
        let (_, mut props) = read_node_props(&self.storage, self.run_id)?;
        props.insert("status".to_string(), Value::String(status.to_string()));
        let props_vec: Vec<(&str, Value)> = props
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        let arena = bumpalo::Bump::new();
        let label = arena.alloc_str("agent_run");
        let len = props_vec.len();
        let arena_props: Vec<(&str, Value)> = props_vec
            .into_iter()
            .map(|(k, v)| (arena.alloc_str(k) as &str, v))
            .collect();
        let properties = sparrow_db::utils::properties::ImmutablePropertiesMap::new(
            len,
            arena_props.into_iter(),
            &arena,
        );
        let node = sparrow_db::utils::items::Node {
            id: self.run_id,
            label,
            version: 2,
            properties: Some(properties),
        };
        let bytes = bincode::serialize(&node).map_err(MemoryError::Serialization)?;
        let mut wtxn = self.storage.graph_env.write_txn().map_err(MemoryError::Heed)?;
        self.storage
            .nodes_db
            .put(&mut wtxn, &self.run_id, &bytes)
            .map_err(MemoryError::Heed)?;
        wtxn.commit().map_err(MemoryError::Heed)?;
        Ok(())
    }
}
```

- [ ] **Step 4: Run test**

```bash
cargo test -p sparrow-memory test_run_record_finding_and_complete -- --nocapture
```

Expected: `ok`

- [ ] **Step 5: Commit**

```bash
git add sparrow-memory/src/run.rs
git commit -m "feat(memory): add RunHandle — record_finding, raise_question, complete, interrupt"
```

---

## Task 7: Recall

**Files:**
- Create: `sparrow-memory/src/recall.rs`

- [ ] **Step 1: Write failing test**

Add to `sparrow-memory/tests/integration.rs`:

```rust
#[test]
fn test_recall_accumulates_across_runs() {
    use sparrow_memory::types::{Finding, Priority};

    let dir = TempDir::new().unwrap();
    let store = MemoryStore::open(MemoryConfig {
        path: dir.path().to_str().unwrap().to_string(),
        db_max_size_gb: Some(1),
    }).unwrap();

    let thread = store.thread("Eye1", "recall-test", "goal").unwrap();

    // Run 1 — add 2 findings, 1 question
    let run1 = thread.start_run().unwrap();
    run1.record_finding(Finding {
        claim: "Finding A".to_string(),
        confidence: 0.9,
        entity_id: None,
        entity_label: None,
        metadata: Default::default(),
    }).unwrap();
    run1.record_finding(Finding {
        claim: "Finding B".to_string(),
        confidence: 0.7,
        entity_id: None,
        entity_label: None,
        metadata: Default::default(),
    }).unwrap();
    run1.raise_question("Open Q", Priority::Medium).unwrap();
    run1.complete("Run 1 done").unwrap();

    // Run 2
    let run2 = thread.start_run().unwrap();
    run2.record_finding(Finding {
        claim: "Finding C".to_string(),
        confidence: 0.8,
        entity_id: None,
        entity_label: None,
        metadata: Default::default(),
    }).unwrap();
    run2.complete("Run 2 done").unwrap();

    let ctx = thread.recall("").unwrap();
    assert_eq!(ctx.recent_summaries.len(), 2);
    assert_eq!(ctx.relevant_findings.len(), 3);
    assert_eq!(ctx.open_questions.len(), 1); // "Open Q" still open
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test -p sparrow-memory test_recall_accumulates_across_runs 2>&1 | tail -5
```

Expected: compile error — `recall` module not found.

- [ ] **Step 3: Create `sparrow-memory/src/recall.rs`**

```rust
use std::sync::Arc;
use sparrow_db::{protocol::value::Value, sparrow_engine::storage_core::SparrowGraphStorage};

use crate::{
    error::MemoryError,
    graph::{ids_from_index, out_neighbors, read_node_props},
    indices::{FINDING_THREAD_ID, QUESTION_THREAD_ID, SUMMARY_THREAD_ID},
    thread::props_to_stored_finding,
    types::{QuestionId, RecallResult, RunId, StoredQuestion, StoredSummary},
};

/// Build a RecallResult for a thread:
/// - All run_summary nodes for the thread (newest first, capped at 5)
/// - All findings for the thread (newest first, capped at 20)
/// - All open questions for the thread
pub fn build_recall(
    storage: Arc<SparrowGraphStorage>,
    thread_id: u128,
) -> Result<RecallResult, MemoryError> {
    let mut result = RecallResult::default();

    // ── Summaries ──────────────────────────────────────────────────────────
    let summary_ids = ids_from_index(&storage, SUMMARY_THREAD_ID, &Value::U128(thread_id))?;
    // v6 UUIDs are time-ordered: higher = newer. Sort descending.
    let mut summary_ids = summary_ids;
    summary_ids.sort_unstable_by(|a, b| b.cmp(a));

    for sid in summary_ids.iter().take(5) {
        let (label, props) = read_node_props(&storage, *sid)?;
        if label != "run_summary" {
            continue;
        }
        result.recent_summaries.push(StoredSummary {
            run_id: RunId(
                props.get("run_id")
                    .and_then(|v| if let Value::U128(id) = v { Some(*id) } else { None })
                    .unwrap_or(0),
            ),
            summary: props.get("summary")
                .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                .unwrap_or_default(),
            finding_count: props.get("finding_count")
                .and_then(|v| if let Value::U32(n) = v { Some(*n) } else { None })
                .unwrap_or(0),
            question_count: props.get("question_count")
                .and_then(|v| if let Value::U32(n) = v { Some(*n) } else { None })
                .unwrap_or(0),
        });
    }

    // ── Findings ───────────────────────────────────────────────────────────
    let mut finding_ids = ids_from_index(&storage, FINDING_THREAD_ID, &Value::U128(thread_id))?;
    finding_ids.sort_unstable_by(|a, b| b.cmp(a));

    for fid in finding_ids.iter().take(20) {
        let (label, props) = read_node_props(&storage, *fid)?;
        if label != "finding" {
            continue;
        }
        result.relevant_findings.push(props_to_stored_finding(*fid, props));
    }

    // ── Open questions ──────────────────────────────────────────────────────
    let question_ids = ids_from_index(&storage, QUESTION_THREAD_ID, &Value::U128(thread_id))?;
    for qid in question_ids {
        let (label, props) = read_node_props(&storage, qid)?;
        if label != "open_question" {
            continue;
        }
        let is_open = props
            .get("status")
            .map(|v| matches!(v, Value::String(s) if s == "open"))
            .unwrap_or(false);
        if !is_open {
            continue;
        }
        result.open_questions.push(StoredQuestion {
            id: QuestionId(qid),
            question: props.get("question")
                .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                .unwrap_or_default(),
            priority: props.get("priority")
                .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                .unwrap_or_default(),
        });
    }

    Ok(result)
}
```

- [ ] **Step 4: Run test**

```bash
cargo test -p sparrow-memory test_recall_accumulates_across_runs -- --nocapture
```

Expected: `ok`

- [ ] **Step 5: Commit**

```bash
git add sparrow-memory/src/recall.rs
git commit -m "feat(memory): add recall — summaries, findings, open questions assembled from secondary indices"
```

---

## Task 8: Aggregate queries and count_distinct test

- [ ] **Step 1: Write failing test**

Add to `sparrow-memory/tests/integration.rs`:

```rust
#[test]
fn test_count_distinct_deduplicates() {
    use sparrow_memory::types::{Finding, Priority};

    let dir = TempDir::new().unwrap();
    let store = MemoryStore::open(MemoryConfig {
        path: dir.path().to_str().unwrap().to_string(),
        db_max_size_gb: Some(1),
    }).unwrap();

    let thread = store.thread("Eye1", "count-test", "goal").unwrap();
    let run = thread.start_run().unwrap();

    let coachee_a = 1001u128;
    let coachee_b = 1002u128;
    let sacred_cow_x = 2001u128;

    // Coachee A holds 3 sacred cows — should count as 1 coachee
    for cow in [2001u128, 2002u128, 2003u128] {
        run.record_finding(Finding {
            claim: format!("coachee_a holds sacred cow {cow}"),
            confidence: 0.9,
            entity_id: Some(coachee_a),
            entity_label: Some("coachee".to_string()),
            metadata: Default::default(),
        }).unwrap();
    }

    // Coachee B holds 1 sacred cow
    run.record_finding(Finding {
        claim: "coachee_b holds sacred cow".to_string(),
        confidence: 0.8,
        entity_id: Some(coachee_b),
        entity_label: Some("coachee".to_string()),
        metadata: Default::default(),
    }).unwrap();

    run.complete("done").unwrap();

    // 4 findings but only 2 distinct coachees
    let count = thread.count_distinct("coachee").unwrap();
    assert_eq!(count, 2);
}
```

- [ ] **Step 2: Run test**

```bash
cargo test -p sparrow-memory test_count_distinct_deduplicates -- --nocapture
```

Expected: `ok` (count_distinct is already implemented in thread.rs from Task 5 — this confirms the end-to-end path)

- [ ] **Step 3: Write findings_for_entity test**

Add to `sparrow-memory/tests/integration.rs`:

```rust
#[test]
fn test_findings_for_entity() {
    use sparrow_memory::types::Finding;

    let dir = TempDir::new().unwrap();
    let store = MemoryStore::open(MemoryConfig {
        path: dir.path().to_str().unwrap().to_string(),
        db_max_size_gb: Some(1),
    }).unwrap();

    let thread = store.thread("Eye1", "entity-test", "goal").unwrap();
    let run = thread.start_run().unwrap();

    let target_id = 5555u128;

    run.record_finding(Finding {
        claim: "finding about target entity".to_string(),
        confidence: 0.9,
        entity_id: Some(target_id),
        entity_label: Some("sacred_cow".to_string()),
        metadata: Default::default(),
    }).unwrap();

    // Finding about a different entity — should not appear
    run.record_finding(Finding {
        claim: "finding about other entity".to_string(),
        confidence: 0.7,
        entity_id: Some(9999u128),
        entity_label: Some("sacred_cow".to_string()),
        metadata: Default::default(),
    }).unwrap();

    run.complete("done").unwrap();

    let findings = thread.findings_for_entity(target_id).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].claim, "finding about target entity");
}
```

- [ ] **Step 4: Run test**

```bash
cargo test -p sparrow-memory test_findings_for_entity -- --nocapture
```

Expected: `ok`

- [ ] **Step 5: Commit**

```bash
git add sparrow-memory/tests/integration.rs
git commit -m "test(memory): add count_distinct deduplication and findings_for_entity tests"
```

---

## Task 9: Full suite and workspace wiring

- [ ] **Step 1: Run all sparrow-memory tests**

```bash
cargo test -p sparrow-memory -- --nocapture 2>&1 | tail -20
```

Expected: all tests pass, no compilation warnings about unused imports.

- [ ] **Step 2: Build the full workspace**

```bash
cargo build --workspace 2>&1 | grep "^error" | head -20
```

Expected: clean build. Fix any errors before proceeding.

- [ ] **Step 3: Check that sparrow-db tests still pass**

```bash
cargo test -p sparrow-db --features lmdb 2>&1 | tail -10
```

Expected: no regressions.

- [ ] **Step 4: Final commit**

```bash
git add -p   # review what's staged
git commit -m "feat(memory): sparrow-memory crate — episodic memory for research agents

Adds a schema-free, SparrowDB-backed research journal:
- MemoryStore::open with secondary index registration
- ThreadHandle: get_or_create, recall, count_distinct, findings_for_entity
- RunHandle: record_finding, raise_question, answer_question, complete, interrupt
- RecallResult: recent summaries + findings + open questions

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Self-review

**Spec coverage:**
- ✅ Persistent research threads — `ThreadHandle::get_or_create`
- ✅ Structured findings with secondary indices — `RunHandle::record_finding` + `FINDING_THREAD_ID` / `FINDING_ENTITY_ID`
- ✅ Open questions carried forward — `RunHandle::raise_question` + `CARRIED` edge in `RunHandle::create`
- ✅ Run summaries — `RunHandle::complete` writes `run_summary` node
- ✅ Aggregate count_distinct — `ThreadHandle::count_distinct`
- ✅ Schema-free (no compiler feature) — `MemoryConfig` sets `schema: None`, `mcp: false`
- ✅ `findings_for_entity` — `ThreadHandle::findings_for_entity`
- ✅ `threads()` listing on `MemoryStore` — **gap**: `MemoryStore::threads()` is in the spec API but not implemented. Add to `store.rs`:

```rust
pub fn threads(&self, agent: &str) -> Result<Vec<crate::types::ThreadSummary>, MemoryError> {
    let rtxn = self.storage.graph_env.read_txn().map_err(MemoryError::Heed)?;
    let arena = bumpalo::Bump::new();
    let mut result = Vec::new();
    for item in self.storage.nodes_db.iter(&rtxn).map_err(MemoryError::Heed)? {
        let (node_id, _) = item.map_err(MemoryError::Heed)?;
        let node = match self.storage.get_node(&rtxn, node_id, &arena) {
            Ok(n) => n,
            Err(_) => continue,
        };
        if node.label != "research_thread" { continue; }
        let agent_match = node.get_property("agent_name")
            .map(|v| matches!(v, Value::String(s) if s == agent))
            .unwrap_or(false);
        if !agent_match { continue; }
        result.push(crate::types::ThreadSummary {
            id: crate::types::ThreadId(node_id),
            name: node.get_property("name")
                .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                .unwrap_or_default(),
            goal: node.get_property("goal")
                .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                .unwrap_or_default(),
            status: node.get_property("status")
                .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                .unwrap_or_default(),
        });
    }
    Ok(result)
}
```

Add this to `store.rs` as part of Task 4 or as an addendum step.

**Placeholder scan:** None found.

**Type consistency:** All IDs (FindingId, QuestionId, RunId, ThreadId) are consistent newtypes wrapping `u128` throughout. `Value::U32` used for `finding_count`/`question_count` — confirm `Value::U32` exists in sparrow-db's enum (it does: `U32(u32)` is in the `Value` definition).
