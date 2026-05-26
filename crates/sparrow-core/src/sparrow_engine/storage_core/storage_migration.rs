use crate::{
    protocol::value::Value,
    sparrow_engine::{
        bm25::{HBM25Config, BM25_SCHEMA_VERSION},
        storage_core::SparrowGraphStorage,
        types::GraphError,
        vector_core::{vector::HVector, VectorCore, ENTRY_POINT_KEY},
    },
    utils::{items::Node, properties::ImmutablePropertiesMap},
};
use bincode::Options;
use itertools::Itertools;
use std::{collections::HashMap, ops::Bound, sync::atomic::Ordering};

use super::metadata::{StorageMetadata, VectorEndianness, NATIVE_VECTOR_ENDIANNESS};

pub fn migrate(storage: &mut SparrowGraphStorage) -> Result<(), GraphError> {
    let mut metadata = {
        let txn = storage.graph_env.read_txn()?;
        StorageMetadata::read(&txn, &storage.metadata_db)?
    };

    loop {
        metadata = match metadata {
            StorageMetadata::PreMetadata => {
                migrate_pre_metadata_to_native_vector_endianness(storage)?
            }
            StorageMetadata::VectorNativeEndianness {
                vector_endianness: NATIVE_VECTOR_ENDIANNESS,
            } => {
                // If the vectors are in the native vector endianness, we're done migrating them
                break;
            }
            StorageMetadata::VectorNativeEndianness {
                vector_endianness: currently_stored_vector_endianness,
            } => convert_vectors_to_native_endianness(currently_stored_vector_endianness, storage)?,
            StorageMetadata::WithSchemaVersion {
                vector_endianness: NATIVE_VECTOR_ENDIANNESS,
                ..
            } => {
                // Vectors are already in native endianness; schema version is informational only
                break;
            }
            StorageMetadata::WithSchemaVersion {
                vector_endianness: currently_stored_vector_endianness,
                ..
            } => convert_vectors_to_native_endianness(currently_stored_vector_endianness, storage)?,
        };
    }

    verify_vectors_and_repair(storage)?;
    remove_orphaned_vector_edges(storage)?;
    migrate_bm25(storage)?;

    // Run HQL schema migrations. Executes before WorkerPool starts so direct write_txn is safe.
    let compiled_transitions: Vec<_> =
        inventory::iter::<crate::sparrow_engine::storage_core::version_info::TransitionSubmission>
            .into_iter()
            .map(|s| s.0.clone())
            .collect();

    crate::sparrow_engine::storage_core::schema_migration::run_schema_migrations(
        storage,
        &compiled_transitions,
    )?;

    Ok(())
}

fn migrate_bm25(storage: &mut SparrowGraphStorage) -> Result<(), GraphError> {
    const BATCH_SIZE: usize = 1024;

    let Some(bm25) = storage.bm25.as_ref() else {
        return Ok(());
    };

    // If BM25 writes are disabled at runtime, skip the startup schema-version
    // rebuild entirely.  The schema version in LMDB is left at its current
    // value; the rebuild will run on the next startup where the flag is cleared.
    // This prevents the 20-30 minute full-graph BM25 rebuild that occurs on
    // every container restart when SPARROW_SKIP_BM25_ON_WRITE=true is set.
    if storage.skip_bm25_writes.load(Ordering::Relaxed) {
        tracing::info!(
            "migrate_bm25: SPARROW_SKIP_BM25_ON_WRITE is set — skipping startup BM25 rebuild"
        );
        return Ok(());
    }

    let current_schema_version = {
        let txn = storage.graph_env.read_txn()?;
        bm25.schema_version(&txn)?
    };

    if current_schema_version == Some(BM25_SCHEMA_VERSION) {
        return Ok(());
    }

    tracing::info!(
        current_version = ?current_schema_version,
        target_version = BM25_SCHEMA_VERSION,
        "migrate_bm25: BM25 schema version mismatch — rebuilding full-text index"
    );

    {
        let mut txn = storage.graph_env.write_txn()?;
        bm25.clear_all(&mut txn)?;
        txn.commit()?;
    }

    let read_txn = storage.graph_env.read_txn()?;
    let mut batch = Vec::with_capacity(BATCH_SIZE);

    for kv in storage.nodes_db.iter(&read_txn)? {
        let (id, value) = kv?;
        batch.push((id, value.to_vec()));

        if batch.len() == BATCH_SIZE {
            rebuild_bm25_batch(storage, bm25, &batch)?;
            batch.clear();
        }
    }

    drop(read_txn);

    if !batch.is_empty() {
        rebuild_bm25_batch(storage, bm25, &batch)?;
    }

    let mut txn = storage.graph_env.write_txn()?;
    bm25.write_schema_version(&mut txn, BM25_SCHEMA_VERSION)?;
    txn.commit()?;

    tracing::info!(
        version = BM25_SCHEMA_VERSION,
        "migrate_bm25: BM25 schema migration complete"
    );

    Ok(())
}

fn rebuild_bm25_batch(
    storage: &SparrowGraphStorage,
    bm25: &HBM25Config,
    batch: &[(u128, Vec<u8>)],
) -> Result<(), GraphError> {
    let arena = bumpalo::Bump::new();
    let mut txn = storage.graph_env.write_txn()?;

    for (id, value) in batch {
        let node = Node::from_bincode_bytes(*id, value, &arena)?;
        if storage.bm25_exclude_labels.contains(node.label) {
            continue;
        }
        if let Some(properties) = node.properties.as_ref() {
            bm25.insert_doc_for_node(&mut txn, *id, properties, node.label)?;
        }
    }

    txn.commit()?;
    Ok(())
}

pub(crate) fn migrate_pre_metadata_to_native_vector_endianness(
    storage: &mut SparrowGraphStorage,
) -> Result<StorageMetadata, GraphError> {
    // In PreMetadata, all vectors are stored as big endian.
    // If we are on a big endian machine, all we need to do is store the metadata.
    // Otherwise, we need to convert all the vectors and then store the metadata.

    let metadata = StorageMetadata::VectorNativeEndianness {
        vector_endianness: NATIVE_VECTOR_ENDIANNESS,
    };

    #[cfg(target_endian = "little")]
    {
        // On little-endian machines, we need to convert from big-endian to little-endian
        convert_all_vectors(VectorEndianness::BigEndian, storage)?;
    }

    convert_all_vector_properties(storage)?;

    // Save the metadata
    let mut txn = storage.graph_env.write_txn()?;
    metadata.save(&mut txn, &storage.metadata_db)?;
    txn.commit()?;

    Ok(metadata)
}

pub(crate) fn convert_vectors_to_native_endianness(
    currently_stored_vector_endianness: VectorEndianness,
    storage: &mut SparrowGraphStorage,
) -> Result<StorageMetadata, GraphError> {
    // Convert all vectors from currently_stored_vector_endianness to native endianness
    convert_all_vectors(currently_stored_vector_endianness, storage)?;

    let metadata = StorageMetadata::VectorNativeEndianness {
        vector_endianness: NATIVE_VECTOR_ENDIANNESS,
    };

    // Save the updated metadata
    let mut txn = storage.graph_env.write_txn()?;
    metadata.save(&mut txn, &storage.metadata_db)?;
    txn.commit()?;

    Ok(metadata)
}

pub(crate) fn convert_all_vectors(
    source_endianness: VectorEndianness,
    storage: &mut SparrowGraphStorage,
) -> Result<(), GraphError> {
    const BATCH_SIZE: usize = 1024;

    let key_arena = bumpalo::Bump::new();
    let batch_bounds = {
        let mut keys = vec![];

        let txn = storage.graph_env.read_txn()?;

        for (i, kv) in storage
            .vectors
            .vectors_db
            .lazily_decode_data()
            .iter(&txn)?
            .enumerate()
        {
            let (key, _) = kv?;

            if i % BATCH_SIZE == 0 {
                let key: &[u8] = key_arena.alloc_slice_copy(key);
                keys.push(key);
            }
        }

        let mut ranges = vec![];
        for (start, end) in keys.iter().copied().tuple_windows() {
            ranges.push((Bound::Included(start), Bound::Excluded(end)));
        }
        ranges.extend(
            keys.last()
                .copied()
                .map(|last_batch_end| (Bound::Included(last_batch_end), Bound::Unbounded)),
        );

        ranges
    };

    for bounds in batch_bounds {
        let arena = bumpalo::Bump::new();

        let mut txn = storage.graph_env.write_txn()?;
        let mut cursor = storage.vectors.vectors_db.range_mut(&mut txn, &bounds)?;

        while let Some((key, value)) = cursor.next().transpose()? {
            if key == ENTRY_POINT_KEY {
                continue;
            }

            let value = convert_vector_endianness(value, source_endianness, &arena)?;

            let success = unsafe { cursor.put_current(key, value)? };
            if !success {
                return Err(GraphError::New("failed to update value in LMDB".into()));
            }
        }
        drop(cursor);

        txn.commit()?;
    }

    Ok(())
}

/// Converts a single vector's endianness by reading f64 values in source endianness
/// and writing them in native endianness. Uses arena for allocations.
pub(crate) fn convert_vector_endianness<'arena>(
    bytes: &[u8],
    source_endianness: VectorEndianness,
    arena: &'arena bumpalo::Bump,
) -> Result<&'arena [u8], GraphError> {
    use std::{alloc, mem, ptr, slice};

    if bytes.is_empty() {
        // We use unsafe stuff below so best not to risk allocating a layout of size zero etc
        return Ok(&[]);
    }

    if !bytes.len().is_multiple_of(mem::size_of::<f64>()) {
        return Err(GraphError::New(
            "Vector data length is not a multiple of f64 size".to_string(),
        ));
    }

    let num_floats = bytes.len() / mem::size_of::<f64>();

    // Allocate space for the converted f64 array in the arena
    let layout = alloc::Layout::array::<f64>(num_floats)
        .map_err(|_| GraphError::New("Failed to create array layout".to_string()))?;

    let data_ptr: ptr::NonNull<u8> = arena.alloc_layout(layout);

    let converted_floats: &'arena [f64] = unsafe {
        let float_ptr: ptr::NonNull<f64> = data_ptr.cast();
        let float_slice = slice::from_raw_parts_mut(float_ptr.as_ptr(), num_floats);

        // Read each f64 in the source endianness and write in native endianness
        for (i, float) in float_slice.iter_mut().enumerate() {
            let start = i * mem::size_of::<f64>();
            let end = start + mem::size_of::<f64>();
            let float_bytes: [u8; 8] = bytes[start..end]
                .try_into()
                .map_err(|_| GraphError::New("Failed to extract f64 bytes".to_string()))?;

            let value = match source_endianness {
                VectorEndianness::BigEndian => f64::from_be_bytes(float_bytes),
                VectorEndianness::LittleEndian => f64::from_le_bytes(float_bytes),
            };

            *float = value;
        }

        slice::from_raw_parts(float_ptr.as_ptr(), num_floats)
    };

    // Convert to bytes using bytemuck
    let result_bytes: &[u8] = bytemuck::cast_slice(converted_floats);

    Ok(result_bytes)
}

pub(crate) fn convert_all_vector_properties(
    storage: &mut SparrowGraphStorage,
) -> Result<(), GraphError> {
    const BATCH_SIZE: usize = 1024;

    let batch_bounds = {
        let txn = storage.graph_env.read_txn()?;
        let mut keys = vec![];

        for (i, kv) in storage
            .vectors
            .vector_properties_db
            .lazily_decode_data()
            .iter(&txn)?
            .enumerate()
        {
            let (key, _) = kv?;

            if i % BATCH_SIZE == 0 {
                keys.push(key);
            }
        }

        let mut ranges = vec![];
        for (start, end) in keys.iter().copied().tuple_windows() {
            ranges.push((Bound::Included(start), Bound::Excluded(end)));
        }
        ranges.extend(
            keys.last()
                .copied()
                .map(|last_batch_end| (Bound::Included(last_batch_end), Bound::Unbounded)),
        );

        ranges
    };

    for bounds in batch_bounds {
        let arena = bumpalo::Bump::new();

        let mut txn = storage.graph_env.write_txn()?;
        let mut cursor = storage
            .vectors
            .vector_properties_db
            .range_mut(&mut txn, &bounds)?;

        while let Some((key, value)) = cursor.next().transpose()? {
            let value = convert_old_vector_properties_to_new_format(value, &arena)?;

            let success = unsafe { cursor.put_current(&key, &value)? };
            if !success {
                return Err(GraphError::New("failed to update value in LMDB".into()));
            }
        }
        drop(cursor);

        txn.commit()?;
    }

    Ok(())
}

pub(crate) fn convert_old_vector_properties_to_new_format(
    property_bytes: &[u8],
    arena: &bumpalo::Bump,
) -> Result<Vec<u8>, GraphError> {
    let mut old_properties: HashMap<String, Value> = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .allow_trailing_bytes()
        .deserialize(property_bytes)?;

    let label = old_properties
        .remove("label")
        .expect("all old vectors should have label");
    let is_deleted = old_properties
        .remove("is_deleted")
        .expect("all old vectors should have deleted");

    let new_properties = ImmutablePropertiesMap::new(
        old_properties.len(),
        old_properties.iter().map(|(k, v)| (k.as_str(), v.clone())),
        arena,
    );

    let new_vector = HVector {
        id: 0u128,
        label: &label.inner_stringify(),
        version: 0,
        deleted: is_deleted == true,
        level: 0,
        distance: None,
        data: &[],
        properties: Some(new_properties),
    };

    new_vector.to_bincode_bytes().map_err(GraphError::from)
}

fn verify_vectors_and_repair(storage: &SparrowGraphStorage) -> Result<(), GraphError> {
    // Verify that all vectors at level > 0 also exist at level 0 and collect ones that need repair
    tracing::info!("verify_vectors: checking HNSW level integrity");
    let vectors_to_repair: Vec<(u128, usize)> = {
        let txn = storage.graph_env.read_txn()?;
        let mut missing = Vec::new();

        for kv in storage.vectors.vectors_db.iter(&txn)? {
            let (key, _) = kv?;
            if key.starts_with(b"v:") && key.len() >= 26 {
                let id = u128::from_be_bytes(key[2..18].try_into().unwrap());
                let level = usize::from_be_bytes(key[18..26].try_into().unwrap());

                if level > 0 {
                    // Check if level 0 exists
                    let level_0_key = VectorCore::vector_key(id, 0);
                    if storage
                        .vectors
                        .vectors_db
                        .get(&txn, &level_0_key)?
                        .is_none()
                    {
                        tracing::error!(
                            vector_id = %uuid::Uuid::from_u128(id),
                            level,
                            "verify_vectors: vector exists at level > 0 but NOT at level 0"
                        );
                        missing.push((id, level));
                    }
                }
            }
        }
        missing
    };

    if !vectors_to_repair.is_empty() {
        tracing::warn!(
            count = vectors_to_repair.len(),
            "verify_vectors: repairing vectors missing their level-0 entry"
        );

        const REPAIR_BATCH_SIZE: usize = 128;

        // Process repairs in batches
        for batch in vectors_to_repair.chunks(REPAIR_BATCH_SIZE) {
            let mut txn = storage.graph_env.write_txn()?;

            let key_arena = bumpalo::Bump::new();

            for &(id, source_level) in batch {
                // Read vector data from source level
                let source_key = VectorCore::vector_key(id, source_level);
                let vector_data: &[u8] = {
                    let key = storage
                        .vectors
                        .vectors_db
                        .get(&txn, &source_key)?
                        .ok_or_else(|| {
                            GraphError::New(format!(
                                "Could not read vector {} at level {source_level} for repair",
                                uuid::Uuid::from_u128(id)
                            ))
                        })?;
                    key_arena.alloc_slice_copy(key)
                };

                // Write to level 0
                let level_0_key = VectorCore::vector_key(id, 0);
                storage
                    .vectors
                    .vectors_db
                    .put(&mut txn, &level_0_key, vector_data)?;
                tracing::info!(
                    vector_id = %uuid::Uuid::from_u128(id),
                    source_level,
                    "verify_vectors: repaired — copied to level 0"
                );
            }

            txn.commit()?;
        }

        tracing::info!(
            repaired = vectors_to_repair.len(),
            "verify_vectors: repair complete"
        );
    } else {
        tracing::info!("verify_vectors: all vectors OK");
    }

    Ok(())
}

fn remove_orphaned_vector_edges(storage: &SparrowGraphStorage) -> Result<(), GraphError> {
    let txn = storage.graph_env.read_txn()?;
    let mut orphaned_edges = Vec::new();

    for kv in storage.vectors.edges_db.iter(&txn)? {
        let (key, _) = kv?;

        // Edge key format: [source_id (16 bytes), level (8 bytes), sink_id (16 bytes)]
        // Total: 40 bytes
        if key.len() != 40 {
            tracing::warn!(
                key_len = key.len(),
                "remove_orphaned_vector_edges: unexpected edge key length, skipping"
            );
            continue;
        }

        // Extract source_id
        let source_id = u128::from_be_bytes(key[0..16].try_into().unwrap());

        // Extract level
        let level = usize::from_be_bytes(key[16..24].try_into().unwrap());

        // Extract sink_id
        let sink_id = u128::from_be_bytes(key[24..40].try_into().unwrap());

        // Check if source vector exists at level 0
        let source_key = VectorCore::vector_key(source_id, 0);
        let source_exists = storage.vectors.vectors_db.get(&txn, &source_key)?.is_some();

        // Check if sink vector exists at level 0
        let sink_key = VectorCore::vector_key(sink_id, 0);
        let sink_exists = storage.vectors.vectors_db.get(&txn, &sink_key)?.is_some();

        if !source_exists || !sink_exists {
            orphaned_edges.push((
                uuid::Uuid::from_u128(source_id),
                level,
                uuid::Uuid::from_u128(sink_id),
            ));
        }
    }

    for chunk in orphaned_edges.into_iter().chunks(64).into_iter() {
        let mut txn = storage.graph_env.write_txn()?;

        for (source_id, level, sink_id) in chunk {
            let edge_key =
                VectorCore::out_edges_key(source_id.as_u128(), level, Some(sink_id.as_u128()));

            storage
                .vectors
                .edges_db
                .get(&txn, &edge_key)?
                .ok_or_else(|| {
                    GraphError::New("edge key doesnt exist when removing orphan".into())
                })?;

            storage.vectors.edges_db.delete(&mut txn, &edge_key)?;
        }

        txn.commit()?;
    }

    Ok(())
}
