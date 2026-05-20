use sparrow_memory::graph::{ids_from_index, out_neighbors, write_edge, write_node, write_node_indexed};
use sparrow_db::{
    protocol::value::Value,
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, storage_methods::StorageMethods, version_info::VersionInfo},
        traversal_core::config::{Config, GraphConfig},
        types::SecondaryIndex,
    },
};
use tempfile::TempDir;

fn open_test_storage() -> (SparrowGraphStorage, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut config = Config::default();
    config.db_max_size_gb = Some(1);
    let vi = VersionInfo::default();
    let storage = SparrowGraphStorage::new(dir.path().to_str().unwrap(), config, vi).unwrap();
    (storage, dir)
}

fn open_test_storage_with_index(idx_name: &str) -> (SparrowGraphStorage, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut config = Config::default();
    config.db_max_size_gb = Some(1);
    config.graph_config = Some(GraphConfig {
        secondary_indices: Some(vec![SecondaryIndex::Index(idx_name.to_string())]),
    });
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

#[test]
fn test_write_edge_and_neighbors() {
    let (storage, _dir) = open_test_storage();
    let from_id = write_node(&storage, "person", vec![]).unwrap();
    let to_id = write_node(&storage, "person", vec![]).unwrap();
    write_edge(&storage, from_id, to_id, "rel").unwrap();
    let neighbors = out_neighbors(&storage, from_id, "rel").unwrap();
    assert!(neighbors.contains(&to_id));
}

#[test]
fn test_indexed_node() {
    let (storage, _dir) = open_test_storage_with_index("test:idx");
    let id_a = write_node_indexed(&storage, "thing", vec![("name", Value::String("a".to_string()))], "test:idx", Value::U128(42)).unwrap();
    let id_b = write_node_indexed(&storage, "thing", vec![("name", Value::String("b".to_string()))], "test:idx", Value::U128(42)).unwrap();
    let ids = ids_from_index(&storage, "test:idx", &Value::U128(42)).unwrap();
    assert!(ids.contains(&id_a), "first node should be in index");
    assert!(ids.contains(&id_b), "second node should be in index");
    assert_eq!(ids.len(), 2, "both nodes under same key, no clobbering");
}
