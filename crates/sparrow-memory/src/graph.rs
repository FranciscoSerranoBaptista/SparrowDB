use bumpalo::Bump;
use heed3::PutFlags;
use sparrow_db::{
    protocol::value::Value,
    sparrow_engine::storage_core::{SparrowGraphStorage, storage_methods::StorageMethods},
    utils::{
        id::v6_uuid,
        items::{Edge, Node},
        label_hash::hash_label,
        properties::ImmutablePropertiesMap,
    },
};

use crate::MemoryError;

/// A list of (property_name, Value) pairs used to build a node.
pub type NodeProps<'a> = Vec<(&'a str, Value)>;

/// Write a new node with the given label and properties in a single write transaction.
/// Returns the new node's u128 ID.
pub fn write_node(
    storage: &SparrowGraphStorage,
    label: &str,
    props: NodeProps<'_>,
) -> Result<u128, MemoryError> {
    // Allocate arena for the node's lifetime during this write.
    let arena = Bump::new();
    let label_ref: &str = arena.alloc_str(label);

    let len = props.len();
    // Allocate prop keys in the arena so they live long enough.
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

    let node = Node {
        id: v6_uuid(),
        label: label_ref,
        version: 1,
        properties,
    };

    let bytes = bincode::serialize(&node).map_err(MemoryError::Serialization)?;

    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;
    storage
        .nodes_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &node.id, &bytes)
        .map_err(MemoryError::Heed)?;
    wtxn.commit().map_err(MemoryError::Heed)?;

    Ok(node.id)
}

/// Write a new node AND register a single secondary index entry — single write transaction.
pub fn write_node_indexed(
    storage: &SparrowGraphStorage,
    label: &str,
    props: NodeProps<'_>,
    index_name: &str,
    index_value: Value,
) -> Result<u128, MemoryError> {
    let arena = Bump::new();
    let label_ref: &str = arena.alloc_str(label);

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

    let node = Node {
        id: v6_uuid(),
        label: label_ref,
        version: 1,
        properties,
    };

    let bytes = bincode::serialize(&node).map_err(MemoryError::Serialization)?;

    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;

    storage
        .nodes_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &node.id, &bytes)
        .map_err(MemoryError::Heed)?;

    let (idx_db, _) = storage
        .secondary_indices
        .get(index_name)
        .ok_or_else(|| MemoryError::IndexNotFound(index_name.to_string()))?;

    let key_bytes = bincode::serialize(&index_value).map_err(MemoryError::Serialization)?;
    idx_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &key_bytes, &node.id)
        .map_err(MemoryError::Heed)?;

    wtxn.commit().map_err(MemoryError::Heed)?;

    Ok(node.id)
}

/// Scan a secondary index for all node IDs matching `key`.
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
    if let Some(iter) = idx_db
        .get_duplicates(&rtxn, key_bytes.as_slice())
        .map_err(MemoryError::Heed)?
    {
        for item in iter {
            let (_, node_id) = item.map_err(MemoryError::Heed)?;
            ids.push(node_id);
        }
    }

    Ok(ids)
}

/// Read a node's properties as `Vec<(String, Value)>`. Returns empty vec if no properties.
pub fn read_node_props(
    storage: &SparrowGraphStorage,
    id: u128,
) -> Result<Vec<(String, Value)>, MemoryError> {
    let arena = Bump::new();
    let rtxn = storage.graph_env.read_txn().map_err(MemoryError::Heed)?;
    let node = storage
        .get_node(&rtxn, id, &arena)
        .map_err(MemoryError::Storage)?;

    let props = match node.properties {
        Some(prop_map) => prop_map.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        None => vec![],
    };

    Ok(props)
}

/// Write a directed edge (no properties) — out and in adjacency entries.
/// Returns the new edge's u128 ID.
pub fn write_edge(
    storage: &SparrowGraphStorage,
    from_id: u128,
    to_id: u128,
    label: &str,
) -> Result<u128, MemoryError> {
    let arena = Bump::new();
    let label_ref: &str = arena.alloc_str(label);

    let version = storage.version_info.get_latest(label);
    let edge = Edge {
        id: v6_uuid(),
        label: label_ref,
        version,
        from_node: from_id,
        to_node: to_id,
        properties: None,
    };

    let edge_bytes = bincode::serialize(&edge).map_err(MemoryError::Serialization)?;

    let label_hash = hash_label(label, None);
    let out_key = SparrowGraphStorage::out_edge_key(&from_id, &label_hash);
    let in_key = SparrowGraphStorage::in_edge_key(&to_id, &label_hash);
    let packed_out = SparrowGraphStorage::pack_edge_data(&edge.id, &to_id);
    let packed_in = SparrowGraphStorage::pack_edge_data(&edge.id, &from_id);

    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;

    storage
        .edges_db
        .put_with_flags(
            &mut wtxn,
            PutFlags::empty(),
            &SparrowGraphStorage::edge_key(&edge.id),
            &edge_bytes,
        )
        .map_err(MemoryError::Heed)?;

    storage
        .out_edges_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &out_key[..], &packed_out[..])
        .map_err(MemoryError::Heed)?;

    storage
        .in_edges_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &in_key[..], &packed_in[..])
        .map_err(MemoryError::Heed)?;

    wtxn.commit().map_err(MemoryError::Heed)?;

    Ok(edge.id)
}

/// Add an existing node to a secondary index without touching the node itself.
pub fn add_to_index(
    storage: &SparrowGraphStorage,
    index_name: &str,
    key: &Value,
    node_id: u128,
) -> Result<(), MemoryError> {
    let (idx_db, _) = storage
        .secondary_indices
        .get(index_name)
        .ok_or_else(|| MemoryError::IndexNotFound(index_name.to_string()))?;
    let key_bytes = bincode::serialize(key).map_err(MemoryError::Serialization)?;
    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;
    idx_db
        .put_with_flags(&mut wtxn, PutFlags::empty(), &key_bytes, &node_id)
        .map_err(MemoryError::Heed)?;
    wtxn.commit().map_err(MemoryError::Heed)?;
    Ok(())
}

/// Remove a specific (key → node_id) entry from a secondary index.
pub fn remove_from_index(
    storage: &SparrowGraphStorage,
    index_name: &str,
    key: &Value,
    node_id: u128,
) -> Result<(), MemoryError> {
    let (idx_db, _) = storage
        .secondary_indices
        .get(index_name)
        .ok_or_else(|| MemoryError::IndexNotFound(index_name.to_string()))?;
    let key_bytes = bincode::serialize(key).map_err(MemoryError::Serialization)?;
    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;
    idx_db
        .delete_one_duplicate(&mut wtxn, key_bytes.as_slice(), &node_id)
        .map_err(MemoryError::Heed)?;
    wtxn.commit().map_err(MemoryError::Heed)?;
    Ok(())
}

/// Get IDs of all nodes reachable via an out-edge of `edge_label` from `from_id`.
pub fn out_neighbors(
    storage: &SparrowGraphStorage,
    from_id: u128,
    edge_label: &str,
) -> Result<Vec<u128>, MemoryError> {
    let label_hash = hash_label(edge_label, None);
    let out_key = SparrowGraphStorage::out_edge_key(&from_id, &label_hash);

    let rtxn = storage.graph_env.read_txn().map_err(MemoryError::Heed)?;
    let mut neighbors = Vec::new();

    if let Some(iter) = storage
        .out_edges_db
        .get_duplicates(&rtxn, &out_key[..])
        .map_err(MemoryError::Heed)?
    {
        for item in iter {
            let (_, packed) = item.map_err(MemoryError::Heed)?;
            if packed.len() >= 32 {
                let to_id = u128::from_be_bytes(packed[16..32].try_into().unwrap());
                neighbors.push(to_id);
            }
        }
    }

    Ok(neighbors)
}
