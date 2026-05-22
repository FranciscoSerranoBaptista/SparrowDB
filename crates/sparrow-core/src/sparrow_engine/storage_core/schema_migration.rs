use crate::{
    protocol::value::Value,
    sparrow_engine::{
        storage_core::{
            SparrowGraphStorage,
            migration_log::{MigrationRecord, MigrationStatus, read_record, write_record},
            metadata::{NATIVE_VECTOR_ENDIANNESS, StorageMetadata},
            version_info::Transition,
        },
        types::GraphError,
    },
    utils::properties::ImmutablePropertiesMap,
};
use std::collections::HashMap;

pub fn run_schema_migrations(
    storage: &mut SparrowGraphStorage,
    transitions: &[Transition],
) -> Result<(), GraphError> {
    if transitions.is_empty() {
        return Ok(());
    }

    // Group transitions by item label and sort by from_version.
    let mut by_label: HashMap<&str, Vec<&Transition>> = HashMap::new();
    for t in transitions {
        by_label.entry(t.item_label).or_default().push(t);
    }
    for chain in by_label.values_mut() {
        chain.sort_by_key(|t| t.from_version);
    }

    // Validate chain contiguity per label.
    for (label, chain) in &by_label {
        for window in chain.windows(2) {
            let prev = window[0];
            let next = window[1];
            if prev.to_version != next.from_version {
                return Err(GraphError::New(format!(
                    "Migration chain gap for '{}': {} → {} is not contiguous with {} → {}",
                    label, prev.from_version, prev.to_version,
                    next.from_version, next.to_version,
                )));
            }
        }
    }

    // Determine latest schema version (highest to_version across all labels).
    let latest_schema_version = transitions
        .iter()
        .map(|t| t.to_version)
        .max()
        .map(|v| format!("v{v}"))
        .unwrap_or_else(|| "v1".to_string());

    // Process each label's chain.
    for (label, chain) in &by_label {
        for transition in chain {
            let migration_name = format!(
                "{}_v{}_v{}",
                label, transition.from_version, transition.to_version
            );
            let checksum = compute_checksum(label, transition.from_version, transition.to_version);

            let existing = {
                let txn = storage.graph_env.read_txn()?;
                read_record(&txn, &storage.migrations_db, &migration_name)?
            };

            match &existing {
                Some(record) if record.status == MigrationStatus::Complete => {
                    if record.checksum != checksum {
                        tracing::warn!(
                            "Migration '{}' checksum mismatch (stored={:#x}, binary={:#x}). Skipping re-run.",
                            migration_name, record.checksum, checksum
                        );
                    }
                    continue;
                }
                _ => {}
            }

            // Mark InProgress before starting.
            {
                let mut wtxn = storage.graph_env.write_txn()?;
                write_record(
                    &mut wtxn,
                    &storage.migrations_db,
                    &migration_name,
                    &MigrationRecord::in_progress(checksum),
                )?;
                wtxn.commit()?;
            }

            run_transition_on_nodes(storage, transition)?;
            run_transition_on_edges(storage, transition)?;

            // Mark Complete.
            {
                let mut wtxn = storage.graph_env.write_txn()?;
                write_record(
                    &mut wtxn,
                    &storage.migrations_db,
                    &migration_name,
                    &MigrationRecord::complete(checksum),
                )?;
                wtxn.commit()?;
            }
        }
    }

    // Update StorageMetadata schema version.
    let current_endianness = {
        let txn = storage.graph_env.read_txn()?;
        StorageMetadata::read(&txn, &storage.metadata_db)?
            .vector_endianness()
            .unwrap_or(NATIVE_VECTOR_ENDIANNESS)
    };
    let mut wtxn = storage.graph_env.write_txn()?;
    StorageMetadata::WithSchemaVersion {
        vector_endianness: current_endianness,
        schema_version: latest_schema_version,
    }
    .save(&mut wtxn, &storage.metadata_db)?;
    wtxn.commit()?;

    Ok(())
}

fn compute_checksum(label: &str, from: u8, to: u8) -> u64 {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };
    let mut h = DefaultHasher::new();
    label.hash(&mut h);
    from.hash(&mut h);
    to.hash(&mut h);
    h.finish()
}

fn run_transition_on_nodes(
    storage: &SparrowGraphStorage,
    transition: &Transition,
) -> Result<(), GraphError> {
    const BATCH_SIZE: usize = 1024;

    let arena = bumpalo::Bump::new();

    // Collect IDs of nodes at from_version.
    let batch_ids: Vec<u128> = {
        let txn = storage.graph_env.read_txn()?;
        let mut ids = Vec::new();
        for kv in storage.nodes_db.iter(&txn)? {
            let (id, bytes) = kv?;
            if let Ok(node) = crate::utils::items::Node::from_bincode_bytes(id, bytes, &arena) {
                if node.version == transition.from_version {
                    ids.push(id);
                }
            }
        }
        ids
    };

    for chunk in batch_ids.chunks(BATCH_SIZE) {
        let arena_batch = bumpalo::Bump::new();
        let mut wtxn = storage.graph_env.write_txn()?;

        for &id in chunk {
            let bytes = match storage.nodes_db.get(&wtxn, &id)? {
                Some(b) => b.to_vec(),
                None => continue,
            };
            let mut node =
                crate::utils::items::Node::from_bincode_bytes(id, &bytes, &arena_batch)?;

            if node.version != transition.from_version {
                continue; // already upgraded by a concurrent write
            }

            let hash_map: HashMap<String, Value> = node
                .properties
                .as_ref()
                .map(|p| p.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
                .unwrap_or_default();

            let new_hash_map = (transition.func)(hash_map);

            let pairs: Vec<(&str, Value)> = new_hash_map
                .iter()
                .map(|(k, v)| {
                    let k_arena: &str = arena_batch.alloc_str(k);
                    (k_arena, v.clone())
                })
                .collect();

            node.properties = Some(ImmutablePropertiesMap::new(
                pairs.len(),
                pairs.into_iter(),
                &arena_batch,
            ));
            node.version = transition.to_version;

            let serialized = node.to_bincode_bytes()?;
            storage.nodes_db.put(&mut wtxn, &id, &serialized)?;
        }

        wtxn.commit()?;
    }

    Ok(())
}

fn run_transition_on_edges(
    storage: &SparrowGraphStorage,
    transition: &Transition,
) -> Result<(), GraphError> {
    const BATCH_SIZE: usize = 1024;

    let arena = bumpalo::Bump::new();

    let batch_ids: Vec<u128> = {
        let txn = storage.graph_env.read_txn()?;
        let mut ids = Vec::new();
        for kv in storage.edges_db.iter(&txn)? {
            let (id, bytes) = kv?;
            if let Ok(edge) = crate::utils::items::Edge::from_bincode_bytes(id, bytes, &arena) {
                if edge.version == transition.from_version {
                    ids.push(id);
                }
            }
        }
        ids
    };

    for chunk in batch_ids.chunks(BATCH_SIZE) {
        let arena_batch = bumpalo::Bump::new();
        let mut wtxn = storage.graph_env.write_txn()?;

        for &id in chunk {
            let bytes = match storage.edges_db.get(&wtxn, &id)? {
                Some(b) => b.to_vec(),
                None => continue,
            };
            let mut edge =
                crate::utils::items::Edge::from_bincode_bytes(id, &bytes, &arena_batch)?;

            if edge.version != transition.from_version {
                continue;
            }

            let hash_map: HashMap<String, Value> = edge
                .properties
                .as_ref()
                .map(|p| p.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
                .unwrap_or_default();

            let new_hash_map = (transition.func)(hash_map);

            let pairs: Vec<(&str, Value)> = new_hash_map
                .iter()
                .map(|(k, v)| {
                    let k_arena: &str = arena_batch.alloc_str(k);
                    (k_arena, v.clone())
                })
                .collect();

            edge.properties = Some(ImmutablePropertiesMap::new(
                pairs.len(),
                pairs.into_iter(),
                &arena_batch,
            ));
            edge.version = transition.to_version;

            let serialized = edge.to_bincode_bytes()?;
            storage.edges_db.put(&mut wtxn, &id, &serialized)?;
        }

        wtxn.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sparrow_engine::{
            storage_core::version_info::VersionInfo,
            traversal_core::config::Config,
        },
        protocol::value::Value,
    };
    use tempfile::TempDir;

    fn make_storage() -> (SparrowGraphStorage, TempDir) {
        let dir = TempDir::new().unwrap();
        let storage = SparrowGraphStorage::new(
            dir.path().to_str().unwrap(),
            Config::default(),
            VersionInfo::default(),
        )
        .unwrap();
        (storage, dir)
    }

    #[test]
    fn no_transitions_is_noop() {
        let (mut storage, _dir) = make_storage();
        let result = run_schema_migrations(&mut storage, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn gap_in_chain_returns_error() {
        fn noop(p: HashMap<String, Value>) -> HashMap<String, Value> {
            p
        }

        let transitions = vec![
            Transition::new("User", 1, 2, noop),
            // Missing v2→v3 — so v3→v4 creates a gap
            Transition::new("User", 3, 4, noop),
        ];

        let (mut storage, _dir) = make_storage();
        let result = run_schema_migrations(&mut storage, &transitions);
        assert!(result.is_err(), "gap in chain must return an error");
    }
}
