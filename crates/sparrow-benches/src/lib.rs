//! Shared fixture helpers for sparrow-benches.
//!
//! `make_engine` creates a minimal in-process SparrowGraphEngine backed by a
//! temp directory. `seed_graph` populates it with `node_count` "person" nodes
//! connected by "knows" edges. Both are meant to be called once per benchmark
//! group (not per iteration).
//!
//! ## Why seed_graph bypasses add_n / add_edge
//!
//! The traversal ops (`add_n`, `add_edge`) write to LMDB using
//! `PutFlags::APPEND`, which requires keys to be strictly ascending in
//! database order.  Node and edge IDs come from `v6_uuid()` (UUID v6,
//! timestamp-based).  When many UUIDs are generated in a tight loop the OS
//! clock may not advance between calls, producing identical or even
//! decreasing u128 values.  This trips `MDB_KEYEXIST` at the LMDB layer.
//!
//! The fix: pre-generate all IDs, sort them, then write directly to the
//! LMDB databases using `put()` (no APPEND constraint).  The LMDB tables
//! still end up with data in the correct format — we just skip the
//! traversal-iterator wrapper.
//!
//! See `docs/superpowers/known-issues.md` for the full write-up.

use sparrow_db::sparrow_engine::traversal_core::{
    SparrowGraphEngine, SparrowGraphEngineOpts,
    config::Config,
};
use sparrow_db::sparrow_engine::storage_core::version_info::VersionInfo;
use sparrow_db::sparrow_engine::storage_core::SparrowGraphStorage;
use sparrow_db::sparrowc::parser::types::{Content, HxFile, Source};
use sparrow_db::utils::id::v6_uuid;
use sparrow_db::utils::items::{Edge, Node};
use sparrow_db::utils::label_hash::hash_label;
use bumpalo::Bump;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Engine factory
// ---------------------------------------------------------------------------

/// Create a minimal SparrowGraphEngine in a temporary directory.
/// The caller must keep `TempDir` alive for the lifetime of the engine.
pub fn make_engine() -> (SparrowGraphEngine, TempDir) {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut config = Config::default();
    config.db_max_size_gb = Some(1);
    let engine = SparrowGraphEngine::new(SparrowGraphEngineOpts {
        path: temp_dir.path().to_str().unwrap().to_string(),
        config,
        version_info: VersionInfo::default(),
        skip_bm25_on_write: None,
    })
    .expect("failed to create SparrowGraphEngine");
    (engine, temp_dir)
}

// ---------------------------------------------------------------------------
// Graph seeding
// ---------------------------------------------------------------------------

/// Generate `count` unique u128 IDs and return them sorted in ascending order.
///
/// Retries if `v6_uuid()` happens to produce duplicates in a burst (extremely
/// rare but possible when the OS clock has coarse resolution).
fn gen_sorted_ids(count: usize) -> Vec<u128> {
    let mut ids: Vec<u128> = (0..count).map(|_| v6_uuid()).collect();
    ids.sort_unstable();
    ids.dedup();
    while ids.len() < count {
        let need = count - ids.len();
        ids.extend((0..need).map(|_| v6_uuid()));
        ids.sort_unstable();
        ids.dedup();
    }
    ids
}

/// Seed `node_count` "person" nodes and edges between consecutive pairs.
///
/// Returns the list of inserted node IDs (sorted ascending). Edges are only
/// inserted where `node_count >= 2`.
///
/// Bypasses the traversal-iterator API (`add_n` / `add_edge`) and writes
/// directly to the LMDB databases to avoid the `PutFlags::APPEND` +
/// non-monotonic UUID issue.  See the module-level doc comment for details.
pub fn seed_graph(storage: &SparrowGraphStorage, node_count: usize) -> Vec<u128> {
    let arena = Bump::new();
    let mut wtxn = storage
        .graph_env
        .write_txn()
        .expect("failed to open write txn");

    // ---- nodes ----
    let node_ids = gen_sorted_ids(node_count);
    let person_label = arena.alloc_str("person");

    for &id in &node_ids {
        let node = Node { id, label: person_label, version: 1, properties: None };
        let bytes = node.to_bincode_bytes().expect("failed to serialize node");
        storage
            .nodes_db
            .put(&mut wtxn, &id, &bytes)
            .expect("failed to write node");
    }

    // ---- edges (chain: node[i] → node[i+1]) ----
    let edge_count = node_ids.len().saturating_sub(1);
    if edge_count > 0 {
        let edge_ids = gen_sorted_ids(edge_count);
        let knows_label = arena.alloc_str("knows");
        let label_hash = hash_label("knows", None);

        for (i, window) in node_ids.windows(2).enumerate() {
            let (from_id, to_id) = (window[0], window[1]);
            let edge_id = edge_ids[i];

            // edges_db: full edge record keyed by edge ID
            let edge = Edge {
                id: edge_id,
                label: knows_label,
                version: 1,
                from_node: from_id,
                to_node: to_id,
                properties: None,
            };
            let bytes = edge.to_bincode_bytes().expect("failed to serialize edge");
            storage
                .edges_db
                .put(&mut wtxn, &edge_id, &bytes)
                .expect("failed to write edge record");

            // out_edges_db: from_node + label_hash → edge_id + to_node
            let out_key = SparrowGraphStorage::out_edge_key(&from_id, &label_hash);
            let out_val = SparrowGraphStorage::pack_edge_data(&edge_id, &to_id);
            storage
                .out_edges_db
                .put(&mut wtxn, &out_key[..], &out_val[..])
                .expect("failed to write out-edge index");

            // in_edges_db: to_node + label_hash → edge_id + from_node
            let in_key = SparrowGraphStorage::in_edge_key(&to_id, &label_hash);
            let in_val = SparrowGraphStorage::pack_edge_data(&edge_id, &from_id);
            storage
                .in_edges_db
                .put(&mut wtxn, &in_key[..], &in_val[..])
                .expect("failed to write in-edge index");
        }
    }

    wtxn.commit().expect("failed to commit seeding txn");
    node_ids
}

// ---------------------------------------------------------------------------
// HQL source constants used by compiler.rs bench
// ---------------------------------------------------------------------------

/// A representative HQL file covering point lookup, traversal, and mutation.
/// The schema and queries are intentionally simple so parser/analyser time
/// is dominated by the compiler pipeline, not schema complexity.
pub const HQL_SOURCE: &str = r#"
N::Person {
    INDEX name: String,
    age: I32,
}

E::Knows {
    From: Person,
    To: Person,
}

QUERY get_person(id: ID) =>
    person <- N<Person>(id)
    RETURN person

QUERY get_friends(id: ID) =>
    person <- N<Person>(id)
    friends <- person::Out<Knows>
    RETURN friends

QUERY add_person(name: String, age: I32) =>
    person <- AddN<Person>({name: name, age: age})
    RETURN person
"#;

/// Wrap a raw HQL string in the `Content` type expected by `SparrowParser::parse_source`.
pub fn make_content(src: &str) -> Content {
    Content {
        content: src.to_string(),
        source: Source::default(),
        files: vec![HxFile {
            name: "bench.hx".to_string(),
            content: src.to_string(),
        }],
    }
}
