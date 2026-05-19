#![cfg(feature = "rocks")]

use std::sync::Arc;

use crate::{
    sparrow_engine::{
        storage_core::SparrowGraphStorage,
        traversal_core::{
            config::Config,
            ops::{
                g::G,
                out::{out::OutAdapter, out_e::OutEdgesAdapter},
                source::{
                    add_e::AddEAdapter, add_n::AddNAdapter, e_from_type::EFromTypeAdapter,
                    n_from_id::NFromIdAdapter, n_from_type::NFromTypeAdapter,
                },
                util::{dedup::DedupAdapter, order::OrderByAdapter, range::RangeAdapter},
                vectors::{insert::InsertVAdapter, search::SearchVAdapter},
            },
        },
        vector_core::vector::HVector,
    },
    props,
};

use bumpalo::Bump;
use tempfile::TempDir;

fn setup_test_db() -> (TempDir, Arc<SparrowGraphStorage>) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().to_str().unwrap();
    let storage = SparrowGraphStorage::new(db_path, Config::default(), Default::default()).unwrap();
    (temp_dir, Arc::new(storage))
}

fn props_option<'arena>(
    arena: &'arena Bump,
    props: Vec<(String, crate::protocol::value::Value)>,
) -> Option<crate::utils::properties::ImmutablePropertiesMap<'arena>> {
    use crate::utils::properties::ImmutablePropertiesMap;
    Some(ImmutablePropertiesMap::new(
        props.len(),
        props.into_iter().map(|(k, v)| {
            let k: &'arena str = arena.alloc_str(&k);
            (k, v)
        }),
        arena,
    ))
}

// ============================================================================
// Node tests
// ============================================================================

#[test]
fn test_rocks_add_and_get_node() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let node = G::new_mut(&storage, &arena, &mut txn)
        .add_n("person", None, None)
        .collect_to_obj()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let fetched = G::new(&storage, &txn, &arena)
        .n_from_id(&node.id())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(fetched.len(), 1);
    assert_eq!(fetched[0].id(), node.id());
}

#[test]
fn test_rocks_n_from_type() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    G::new_mut(&storage, &arena, &mut txn)
        .add_n("person", None, None)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("person", None, None)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("company", None, None)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let persons = G::new(&storage, &txn, &arena)
        .n_from_type("person")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(persons.len(), 2);

    let companies = G::new(&storage, &txn, &arena)
        .n_from_type("company")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(companies.len(), 1);
}

// ============================================================================
// Edge tests
// ============================================================================

#[test]
fn test_rocks_add_and_traverse_edge() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let src = G::new_mut(&storage, &arena, &mut txn)
        .add_n("person", None, None)
        .collect_to_obj()
        .unwrap();
    let dst = G::new_mut(&storage, &arena, &mut txn)
        .add_n("person", None, None)
        .collect_to_obj()
        .unwrap();

    let edge = G::new_mut(&storage, &arena, &mut txn)
        .add_edge("knows", None, src.id(), dst.id(), false)
        .collect_to_obj()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();

    // Traverse from source through out_node
    let reached = G::new(&storage, &txn, &arena)
        .n_from_id(&src.id())
        .out_node("knows")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(reached.len(), 1);
    assert_eq!(reached[0].id(), dst.id());

    // Traverse out edges
    let edges = G::new(&storage, &txn, &arena)
        .n_from_id(&src.id())
        .out_e("knows")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].id(), edge.id());
}

#[test]
fn test_rocks_e_from_type() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let a = G::new_mut(&storage, &arena, &mut txn)
        .add_n("node", None, None)
        .collect_to_obj()
        .unwrap();
    let b = G::new_mut(&storage, &arena, &mut txn)
        .add_n("node", None, None)
        .collect_to_obj()
        .unwrap();
    let c = G::new_mut(&storage, &arena, &mut txn)
        .add_n("node", None, None)
        .collect_to_obj()
        .unwrap();

    G::new_mut(&storage, &arena, &mut txn)
        .add_edge("knows", None, a.id(), b.id(), false)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_edge("knows", None, b.id(), c.id(), false)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_edge("likes", None, a.id(), c.id(), false)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let knows_edges = G::new(&storage, &txn, &arena)
        .e_from_type("knows")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(knows_edges.len(), 2);

    let likes_edges = G::new(&storage, &txn, &arena)
        .e_from_type("likes")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(likes_edges.len(), 1);
}

#[test]
fn test_rocks_unique_edge_check() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let src = G::new_mut(&storage, &arena, &mut txn)
        .add_n("person", None, None)
        .collect_to_obj()
        .unwrap();
    let dst = G::new_mut(&storage, &arena, &mut txn)
        .add_n("person", None, None)
        .collect_to_obj()
        .unwrap();

    // First edge (unique enforcement) — should succeed
    G::new_mut(&storage, &arena, &mut txn)
        .add_edge("knows", None, src.id(), dst.id(), true)
        .collect_to_obj()
        .unwrap();

    txn.commit().unwrap();

    // Second identical edge with unique check — should fail
    let mut txn2 = storage.write_txn().unwrap();
    let result = G::new_mut(&storage, &arena, &mut txn2)
        .add_edge("knows", None, src.id(), dst.id(), true)
        .collect_to_obj();
    assert!(result.is_err(), "Expected duplicate edge to fail");
}

// ============================================================================
// Utility ops: order, dedup, range
// ============================================================================

#[test]
fn test_rocks_order_by_asc() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let n1 = G::new_mut(&storage, &arena, &mut txn)
        .add_n("item", props_option(&arena, props! { "score" => 30 }), None)
        .collect_to_obj()
        .unwrap();
    let n2 = G::new_mut(&storage, &arena, &mut txn)
        .add_n("item", props_option(&arena, props! { "score" => 10 }), None)
        .collect_to_obj()
        .unwrap();
    let n3 = G::new_mut(&storage, &arena, &mut txn)
        .add_n("item", props_option(&arena, props! { "score" => 20 }), None)
        .collect_to_obj()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let sorted = G::new(&storage, &txn, &arena)
        .n_from_type("item")
        .order_by_asc("score")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(sorted.len(), 3);
    assert_eq!(sorted[0].id(), n2.id()); // 10
    assert_eq!(sorted[1].id(), n3.id()); // 20
    assert_eq!(sorted[2].id(), n1.id()); // 30
}

#[test]
fn test_rocks_order_by_desc() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let n1 = G::new_mut(&storage, &arena, &mut txn)
        .add_n("item", props_option(&arena, props! { "score" => 30 }), None)
        .collect_to_obj()
        .unwrap();
    let n2 = G::new_mut(&storage, &arena, &mut txn)
        .add_n("item", props_option(&arena, props! { "score" => 10 }), None)
        .collect_to_obj()
        .unwrap();
    let n3 = G::new_mut(&storage, &arena, &mut txn)
        .add_n("item", props_option(&arena, props! { "score" => 20 }), None)
        .collect_to_obj()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let sorted = G::new(&storage, &txn, &arena)
        .n_from_type("item")
        .order_by_desc("score")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(sorted.len(), 3);
    assert_eq!(sorted[0].id(), n1.id()); // 30
    assert_eq!(sorted[1].id(), n3.id()); // 20
    assert_eq!(sorted[2].id(), n2.id()); // 10
}

#[test]
fn test_rocks_dedup() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let shared = G::new_mut(&storage, &arena, &mut txn)
        .add_n("node", None, None)
        .collect_to_obj()
        .unwrap();
    let src1 = G::new_mut(&storage, &arena, &mut txn)
        .add_n("node", None, None)
        .collect_to_obj()
        .unwrap();
    let src2 = G::new_mut(&storage, &arena, &mut txn)
        .add_n("node", None, None)
        .collect_to_obj()
        .unwrap();

    // Both src1 and src2 point to shared
    G::new_mut(&storage, &arena, &mut txn)
        .add_edge("rel", None, src1.id(), shared.id(), false)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_edge("rel", None, src2.id(), shared.id(), false)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();

    // Without dedup: 2 results (both pointing to shared)
    let without = G::new(&storage, &txn, &arena)
        .n_from_type("node")
        .out_node("rel")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(without.len(), 2);

    // With dedup: 1 result
    let with_dedup = G::new(&storage, &txn, &arena)
        .n_from_type("node")
        .out_node("rel")
        .dedup()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(with_dedup.len(), 1);
    assert_eq!(with_dedup[0].id(), shared.id());
}

#[test]
fn test_rocks_range() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    for _ in 0..5 {
        G::new_mut(&storage, &arena, &mut txn)
            .add_n("item", None, None)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    }

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let ranged = G::new(&storage, &txn, &arena)
        .n_from_type("item")
        .range(1, 3) // indices 1 and 2
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(ranged.len(), 2);
}

// ============================================================================
// Vector tests
// ============================================================================

#[test]
fn test_rocks_insert_and_search_vector() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    type FnTy = fn(&HVector, &rocksdb::Transaction<rocksdb::TransactionDB>) -> bool;

    let v1 = G::new_mut(&storage, &arena, &mut txn)
        .insert_v::<FnTy>(&[1.0, 0.0, 0.0], "embedding", None)
        .collect_to_obj()
        .unwrap();

    let v2 = G::new_mut(&storage, &arena, &mut txn)
        .insert_v::<FnTy>(&[0.0, 1.0, 0.0], "embedding", None)
        .collect_to_obj()
        .unwrap();

    let _v3 = G::new_mut(&storage, &arena, &mut txn)
        .insert_v::<FnTy>(&[0.0, 0.0, 1.0], "embedding", None)
        .collect_to_obj()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();

    // Search nearest to [1.0, 0.0, 0.0] → should return v1 first
    let results = G::new(&storage, &txn, &arena)
        .search_v::<FnTy, _>(&[1.0, 0.0, 0.0], 2, "embedding", None)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].id(), v1.id());

    // Search nearest to [0.0, 1.0, 0.0] → should return v2 first
    let results2 = G::new(&storage, &txn, &arena)
        .search_v::<FnTy, _>(&[0.0, 1.0, 0.0], 1, "embedding", None)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(results2.len(), 1);
    assert_eq!(results2[0].id(), v2.id());
}

#[test]
fn test_rocks_vector_order_by() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    type FnTy = fn(&HVector, &rocksdb::Transaction<rocksdb::TransactionDB>) -> bool;

    let v_low = G::new_mut(&storage, &arena, &mut txn)
        .insert_v::<FnTy>(
            &[1.0, 0.0],
            "vec",
            props_option(&arena, props! { "rank" => 10 }),
        )
        .collect_to_obj()
        .unwrap();

    let v_high = G::new_mut(&storage, &arena, &mut txn)
        .insert_v::<FnTy>(
            &[1.0, 0.0],
            "vec",
            props_option(&arena, props! { "rank" => 99 }),
        )
        .collect_to_obj()
        .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let sorted = G::new(&storage, &txn, &arena)
        .search_v::<FnTy, _>(&[1.0, 0.0], 10, "vec", None)
        .order_by_asc("rank")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(sorted.len(), 2);
    assert_eq!(sorted[0].id(), v_low.id());
    assert_eq!(sorted[1].id(), v_high.id());
}

// ============================================================================
// Properties test
// ============================================================================

#[test]
fn test_rocks_node_with_properties() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    let node = G::new_mut(
        &storage,
        &arena,
        &mut txn,
    )
    .add_n(
        "person",
        props_option(&arena, props! { "name" => "Alice", "age" => 30 }),
        None,
    )
    .collect_to_obj()
    .unwrap();

    txn.commit().unwrap();

    let txn = storage.read_txn().unwrap();
    let fetched = G::new(&storage, &txn, &arena)
        .n_from_id(&node.id())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(fetched.len(), 1);
    let fetched_node = &fetched[0];
    assert_eq!(
        fetched_node.get_property("name").map(|v| v.as_str()),
        Some("Alice")
    );
}

// ============================================================================
// Transaction isolation test
// ============================================================================

#[test]
fn test_rocks_uncommitted_not_visible() {
    let (_temp_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.write_txn().unwrap();

    G::new_mut(&storage, &arena, &mut txn)
        .add_n("thing", None, None)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    // Don't commit — read in a separate txn
    let read_txn = storage.read_txn().unwrap();
    let things = G::new(&storage, &read_txn, &arena)
        .n_from_type("thing")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    // RocksDB optimistic transactions: uncommitted writes not visible to other txns
    assert_eq!(things.len(), 0);

    // Now commit and verify it's visible
    txn.commit().unwrap();
    let read_txn2 = storage.read_txn().unwrap();
    let things2 = G::new(&storage, &read_txn2, &arena)
        .n_from_type("thing")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(things2.len(), 1);
}
