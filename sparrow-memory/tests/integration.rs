use sparrow_memory::graph::write_node;
use sparrow_db::{
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, storage_methods::StorageMethods, version_info::VersionInfo},
        traversal_core::config::Config,
    },
    protocol::value::Value,
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
