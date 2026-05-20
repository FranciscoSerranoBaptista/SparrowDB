use std::collections::HashMap;

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

/// Write a new node AND register secondary index entries — single write transaction.
/// `index_entries` is a slice of `(index_name, key_value)` pairs.
pub fn write_node_indexed(
    storage: &SparrowGraphStorage,
    label: &str,
    props: NodeProps<'_>,
    index_entries: &[(&str, Value)],
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

    for (index_name, key_value) in index_entries {
        let (idx_db, _) = storage
            .secondary_indices
            .get(*index_name)
            .ok_or_else(|| MemoryError::IndexNotFound(index_name.to_string()))?;

        let key_bytes =
            bincode::serialize(key_value).map_err(MemoryError::Serialization)?;
        idx_db
            .put(&mut wtxn, &key_bytes, &node.id)
            .map_err(MemoryError::Heed)?;
    }

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
    for item in idx_db
        .prefix_iter(&rtxn, &key_bytes)
        .map_err(MemoryError::Heed)?
    {
        let (_, node_id) = item.map_err(MemoryError::Heed)?;
        ids.push(node_id);
    }

    Ok(ids)
}

/// Read a node, returning `(label, properties HashMap)`. Arena is internal.
pub fn read_node_props(
    storage: &SparrowGraphStorage,
    id: u128,
) -> Result<(String, HashMap<String, Value>), MemoryError> {
    let arena = Bump::new();
    let rtxn = storage.graph_env.read_txn().map_err(MemoryError::Heed)?;
    let node = storage
        .get_node(&rtxn, id, &arena)
        .map_err(MemoryError::Storage)?;

    let label = node.label.to_string();
    let mut props = HashMap::new();
    if let Some(prop_map) = node.properties {
        for (k, v) in prop_map.iter() {
            props.insert(k.to_string(), v.clone());
        }
    }

    Ok((label, props))
}

/// Write a directed edge (no properties) — out and in adjacency entries.
/// Returns the new edge's u128 ID.
pub fn write_edge(
    storage: &SparrowGraphStorage,
    label: &str,
    from_id: u128,
    to_id: u128,
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
        .put(&mut wtxn, &out_key[..], &packed_out[..])
        .map_err(MemoryError::Heed)?;

    storage
        .in_edges_db
        .put(&mut wtxn, &in_key[..], &packed_in[..])
        .map_err(MemoryError::Heed)?;

    wtxn.commit().map_err(MemoryError::Heed)?;

    Ok(edge.id)
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
