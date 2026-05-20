use super::test_utils::props_option;
use std::sync::Arc;

use heed3::RoTxn;

use crate::sparrow_engine::{
    storage_core::SparrowGraphStorage,
    traversal_core::{
        ops::{
            g::G,
            source::{
                add_n::AddNAdapter, n_from_type::NFromTypeAdapter,
                v_from_type::VFromTypeAdapter,
            },
            util::map::MapAdapter,
            vectors::insert::InsertVAdapter,
        },
        traversal_value::TraversalValue,
    },
    types::GraphError,
    vector_core::vector::HVector,
};
use crate::{props, protocol::value::Value};
use bumpalo::Bump;
use tempfile::TempDir;

type Filter = fn(&HVector, &RoTxn) -> bool;

fn setup_test_db() -> (TempDir, Arc<SparrowGraphStorage>) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().to_str().unwrap();
    let storage = SparrowGraphStorage::new(
        db_path,
        crate::sparrow_engine::traversal_core::config::Config::default(),
        Default::default(),
    )
    .unwrap();
    (temp_dir, Arc::new(storage))
}

// ── Task 1 regression: same-type I128 arms ──────────────────────────────────

#[test]
fn test_i128_add_same_type_at_boundary() {
    // Before the fix, I128 + I128 fell through to the cross-type signed arm
    // and returned Value::I64, silently truncating i128::MAX + 1.
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("num", props_option(&arena, props! { "v" => Value::I128(i128::MAX) }), None)
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let v = node.get_property("v").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(v.clone() + Value::I128(1)))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0] {
        TraversalValue::Value(Value::I128(v)) => {
            assert_eq!(*v, i128::MAX.wrapping_add(1))
        }
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_i128_mul_same_type_at_boundary() {
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("num", props_option(&arena, props! { "v" => Value::I128(i128::MAX) }), None)
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let v = node.get_property("v").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(v.clone() * Value::I128(2)))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0] {
        TraversalValue::Value(Value::I128(v)) => {
            assert_eq!(*v, i128::MAX.wrapping_mul(2))
        }
        other => panic!("expected I128, got {other:?}"),
    }
}

// ── Task 2 regression: cross-type signed → I128 (not I64) ───────────────────

#[test]
fn test_cross_type_signed_i128_plus_i8_preserves_variant() {
    // Before the fix, I128(x) + I8(y) called to_i64() and returned Value::I64,
    // silently truncating any I128 value outside i64::MAX.
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("num", props_option(&arena, props! { "v" => Value::I128(i128::MAX) }), None)
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let v = node.get_property("v").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(v.clone() + Value::I8(0)))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0] {
        TraversalValue::Value(Value::I128(v)) => assert_eq!(*v, i128::MAX),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_cross_type_signed_i64_plus_i32_returns_i128() {
    // All cross-type signed arithmetic now promotes to I128.
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("num", props_option(&arena, props! { "v" => Value::I64(100) }), None)
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let v = node.get_property("v").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(v.clone() + Value::I32(50)))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0] {
        TraversalValue::Value(Value::I128(v)) => assert_eq!(*v, 150),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_cross_type_signed_div_and_rem_preserve_i128() {
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("num", props_option(&arena, props! { "v" => Value::I128(i128::MAX) }), None)
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let div_results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let v = node.get_property("v").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(v.clone() / Value::I32(2)))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    match &div_results[0] {
        TraversalValue::Value(Value::I128(v)) => assert_eq!(*v, i128::MAX / 2),
        other => panic!("div: expected I128, got {other:?}"),
    }

    drop(txn);
    let txn = storage.graph_env.read_txn().unwrap();
    let rem_results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let v = node.get_property("v").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(v.clone() % Value::I32(3)))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    match &rem_results[0] {
        TraversalValue::Value(Value::I128(v)) => assert_eq!(*v, i128::MAX % 3),
        other => panic!("rem: expected I128, got {other:?}"),
    }
}

// ── Task 3 regression: abs, min, max ────────────────────────────────────────

#[test]
fn test_abs_i128_via_traversal() {
    // Before the fix, Value::I128.abs() hit `_ => panic!("abs requires numeric value")`.
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("num", props_option(&arena, props! { "v" => Value::I128(-99) }), None)
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let v = node.get_property("v").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(v.abs()))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0] {
        TraversalValue::Value(Value::I128(v)) => assert_eq!(*v, 99),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_min_cross_type_integer_via_traversal() {
    // Before the fix, min() between different integer types promoted to F64.
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("num", props_option(&arena, props! { "a" => Value::I32(5), "b" => Value::I64(10) }), None)
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let a = node.get_property("a").ok_or(GraphError::NodeNotFound)?;
                let b = node.get_property("b").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(a.min(b)))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0] {
        TraversalValue::Value(Value::F64(_)) => panic!("cross-type integer min must not produce F64"),
        TraversalValue::Value(v) => assert_eq!(*v, Value::I32(5)),
        other => panic!("expected Value, got {other:?}"),
    }
}

#[test]
fn test_max_cross_type_integer_via_traversal() {
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();
    G::new_mut(&storage, &arena, &mut txn)
        .add_n("num", props_option(&arena, props! { "a" => Value::I32(5), "b" => Value::I64(10) }), None)
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let results = G::new(&storage, &txn, &arena)
        .n_from_type("num")
        .map_traversal(|tv, _| {
            if let TraversalValue::Node(node) = tv {
                let a = node.get_property("a").ok_or(GraphError::NodeNotFound)?;
                let b = node.get_property("b").ok_or(GraphError::NodeNotFound)?;
                Ok(TraversalValue::Value(a.max(b)))
            } else {
                Err(GraphError::New("expected node".into()))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0] {
        TraversalValue::Value(Value::F64(_)) => panic!("cross-type integer max must not produce F64"),
        TraversalValue::Value(v) => assert_eq!(*v, Value::I64(10)),
        other => panic!("expected Value, got {other:?}"),
    }
}

// ── Realistic I64 property case (vector count metadata) ─────────────────────

#[test]
fn test_i64_count_property_arithmetic_on_vector_node() {
    // Vector nodes commonly carry integer metadata (counts, sizes, ranks) stored as
    // Value::I64 — the type JSON integers deserialize to. Before the cross-type
    // signed fix, I64(count) + I32(delta) silently truncated to I64 via to_i64().
    // Now it returns I128. This test exercises that path on a real vector node.
    let (_dir, storage) = setup_test_db();
    let arena = Bump::new();
    let mut txn = storage.graph_env.write_txn().unwrap();

    use crate::utils::properties::ImmutablePropertiesMap;
    let props_map = ImmutablePropertiesMap::new(
        1,
        [("count", Value::I64(42))].iter().map(|(k, v)| {
            (arena.alloc_str(k) as &str, v.clone())
        }),
        &arena,
    );
    G::new_mut(&storage, &arena, &mut txn)
        .insert_v::<Filter>(&[1.0, 0.0, 0.0], "doc", Some(props_map))
        .collect_to_obj()
        .unwrap();
    txn.commit().unwrap();

    let txn = storage.graph_env.read_txn().unwrap();
    let results = G::new(&storage, &txn, &arena)
        .v_from_type("doc", true)
        .map_traversal(|tv, _| {
            let count = tv.get_property("count").ok_or(GraphError::NodeNotFound)?;
            Ok(TraversalValue::Value(count.clone() + Value::I32(1)))
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0] {
        TraversalValue::Value(Value::I128(v)) => assert_eq!(*v, 43),
        other => panic!("expected I128(43), got {other:?}"),
    }
}
