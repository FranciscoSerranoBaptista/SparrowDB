//! Shared fixture helpers for sparrow-benches.
//!
//! `make_engine` creates a minimal in-process SparrowGraphEngine backed by a
//! temp directory. `seed_graph` populates it with `node_count` "person" nodes
//! connected by "knows" edges. Both are meant to be called once per benchmark
//! group (not per iteration).

use sparrow_db::sparrow_engine::traversal_core::{
    SparrowGraphEngine, SparrowGraphEngineOpts,
    config::Config,
    ops::{
        g::G,
        source::{add_e::AddEAdapter, add_n::AddNAdapter},
    },
};
use sparrow_db::sparrow_engine::storage_core::version_info::VersionInfo;
use sparrow_db::sparrow_engine::storage_core::SparrowGraphStorage;
use sparrow_db::sparrowc::parser::types::{Content, HxFile, Source};
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
    })
    .expect("failed to create SparrowGraphEngine");
    (engine, temp_dir)
}

// ---------------------------------------------------------------------------
// Graph seeding
// ---------------------------------------------------------------------------

/// Seed `node_count` "person" nodes and edges between consecutive pairs.
///
/// Returns the list of inserted node IDs. Edges are only inserted where
/// `node_count >= 2`.
///
/// Calling `write_txn()` directly is intentional — benchmarks run without a
/// WorkerPool, so there is no concurrent writer and the single-writer
/// invariant is satisfied.
pub fn seed_graph(storage: &SparrowGraphStorage, node_count: usize) -> Vec<u128> {
    let arena = Bump::new();
    let mut wtxn = storage
        .graph_env
        .write_txn()
        .expect("failed to open write txn");

    let mut ids: Vec<u128> = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let node = G::new_mut(storage, &arena, &mut wtxn)
            .add_n("person", None, None)
            .collect_to_obj()
            .expect("add_n failed");
        ids.push(node.id());
    }

    for window in ids.windows(2) {
        G::new_mut(storage, &arena, &mut wtxn)
            .add_edge("knows", None, window[0], window[1], false)
            .collect_to_obj()
            .expect("add_edge failed");
    }

    wtxn.commit().expect("failed to commit seeding txn");
    ids
}

// ---------------------------------------------------------------------------
// HQL source constants used by compiler.rs bench
// ---------------------------------------------------------------------------

/// A representative HQL file covering point lookup, traversal, and mutation.
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
