use crate::sparrow_engine::bm25::lmdb_bm25::{BM25, BM25Flatten};
use crate::sparrow_engine::vector_core::HNSW;
use std::sync::atomic::Ordering;
use crate::{
    sparrow_engine::{
        traversal_core::{traversal_iter::RwTraversalIterator, traversal_value::TraversalValue},
        types::GraphError,
    },
    utils::{id::v6_uuid, items::Node, properties::ImmutablePropertiesMap},
};
pub trait AddNAdapter<'db, 'arena, 'txn, 's>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    fn add_n(
        self,
        label: &'arena str,
        properties: Option<ImmutablePropertiesMap<'arena>>,
        secondary_indices: Option<&'s [&str]>,
    ) -> RwTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >;

    fn add_n_with_vectors(
        self,
        label: &'arena str,
        properties: Option<ImmutablePropertiesMap<'arena>>,
        secondary_indices: Option<&'s [&str]>,
        vector_inserts: Option<&'s [(&'arena str, &'s [f32])]>,
    ) -> RwTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >;
}

// LMDB Implementation
impl<'db, 'arena, 'txn, 's, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    AddNAdapter<'db, 'arena, 'txn, 's> for RwTraversalIterator<'db, 'arena, 'txn, I>
{
    fn add_n(
        self,
        label: &'arena str,
        properties: Option<ImmutablePropertiesMap<'arena>>,
        secondary_indices: Option<&'s [&str]>,
    ) -> RwTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        let node = Node {
            id: v6_uuid(),
            label,
            version: 1,
            properties,
        };
        let secondary_indices = secondary_indices.unwrap_or(&[]).to_vec();
        let mut result: Result<TraversalValue, GraphError> = Ok(TraversalValue::Empty);

        match bincode::serialize(&node) {
            Ok(bytes) => {
                if let Err(e) = self.storage.nodes_db.put(self.txn, &node.id, &bytes) {
                    result = Err(GraphError::from(e));
                }
            }
            Err(e) => result = Err(GraphError::from(e)),
        }

        for index in secondary_indices {
            // Keys in the HashMap are "TypeName:field_name" since the
            // global-namespace fix.  Qualify with the node label.
            let qualified = format!("{label}:{index}");
            match self.storage.secondary_indices.get(qualified.as_str()) {
                Some(db) => {
                    let key = match node.get_property(index) {
                        Some(value) => value,
                        None => continue,
                    };
                    // look into if there is a way to serialize to a slice
                    match bincode::serialize(&key) {
                        Ok(serialized) => {
                            // For unique indices, reject duplicate values before writing.
                            if matches!(db.1, crate::sparrow_engine::types::SecondaryIndex::Unique(_)) {
                                match db.0.get(self.txn, &serialized) {
                                    Ok(Some(_)) => {
                                        result = Err(GraphError::DuplicateKey(format!(
                                            "Unique index '{index}' already contains this value"
                                        )));
                                        continue;
                                    }
                                    Err(e) => {
                                        result = Err(GraphError::from(e));
                                        continue;
                                    }
                                    Ok(None) => {}
                                }
                            }

                            if let Err(e) = db.0.put(self.txn, &serialized, &node.id) {
                                tracing::error!(
                                    error = ?e,
                                    "add_n: failed to write secondary index entry"
                                );
                                result = Err(GraphError::from(e));
                            }
                        }
                        Err(e) => result = Err(GraphError::from(e)),
                    }
                }
                None => {
                    result = Err(GraphError::New(format!(
                        "Secondary Index {index} not found"
                    )));
                }
            }
        }

        if let Some(bm25) = self.storage.bm25.as_ref().filter(|_| {
            !self.storage.skip_bm25_writes.load(Ordering::Acquire)
                && !self.storage.bm25_exclude_labels.contains(node.label)
        }) && let Some(props) = node.properties.as_ref()
        {
            let mut data = props.flatten_bm25();
            data.push_str(node.label);
            if let Err(e) = bm25.insert_doc(self.txn, node.id, &data) {
                result = Err(e);
            }
        }

        if result.is_ok() {
            result = Ok(TraversalValue::Node(node));
        }
        // Preserve the specific error (e.g. DuplicateKey) rather than replacing it.

        RwTraversalIterator {
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
            inner: std::iter::once(result),
        }
    }

    fn add_n_with_vectors(
        self,
        label: &'arena str,
        properties: Option<ImmutablePropertiesMap<'arena>>,
        secondary_indices: Option<&'s [&str]>,
        vector_inserts: Option<&'s [(&'arena str, &'s [f32])]>,
    ) -> RwTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        let node = Node {
            id: v6_uuid(),
            label,
            version: 1,
            properties,
        };
        let secondary_indices = secondary_indices.unwrap_or(&[]).to_vec();
        let mut result: Result<TraversalValue, GraphError> = Ok(TraversalValue::Empty);

        match bincode::serialize(&node) {
            Ok(bytes) => {
                if let Err(e) = self.storage.nodes_db.put(self.txn, &node.id, &bytes) {
                    result = Err(GraphError::from(e));
                }
            }
            Err(e) => result = Err(GraphError::from(e)),
        }

        for index in secondary_indices {
            // Keys in the HashMap are "TypeName:field_name" since the
            // global-namespace fix.  Qualify with the node label.
            let qualified = format!("{label}:{index}");
            match self.storage.secondary_indices.get(qualified.as_str()) {
                Some(db) => {
                    let key = match node.get_property(index) {
                        Some(value) => value,
                        None => continue,
                    };
                    match bincode::serialize(&key) {
                        Ok(serialized) => {
                            if matches!(db.1, crate::sparrow_engine::types::SecondaryIndex::Unique(_)) {
                                match db.0.get(self.txn, &serialized) {
                                    Ok(Some(_)) => {
                                        result = Err(GraphError::DuplicateKey(format!(
                                            "Unique index '{index}' already contains this value"
                                        )));
                                        continue;
                                    }
                                    Err(e) => {
                                        result = Err(GraphError::from(e));
                                        continue;
                                    }
                                    Ok(None) => {}
                                }
                            }

                            if let Err(e) = db.0.put(self.txn, &serialized, &node.id) {
                                result = Err(GraphError::from(e));
                            }
                        }
                        Err(e) => result = Err(GraphError::from(e)),
                    }
                }
                None => {
                    result = Err(GraphError::New(format!(
                        "Secondary Index {index} not found"
                    )));
                }
            }
        }

        if let Some(bm25) = self.storage.bm25.as_ref().filter(|_| {
            !self.storage.skip_bm25_writes.load(Ordering::Acquire)
                && !self.storage.bm25_exclude_labels.contains(node.label)
        }) && let Some(props) = node.properties.as_ref()
        {
            let mut data = props.flatten_bm25();
            data.push_str(node.label);
            if let Err(e) = bm25.insert_doc(self.txn, node.id, &data) {
                result = Err(e);
            }
        }

        // Insert vector fields into HNSW, gated on prior success
        if result.is_ok() {
            if let Some(inserts) = vector_inserts {
                for (vec_label, f32_data) in inserts {
                    // Convert f32 slice to arena-allocated f64 slice
                    let f64_slice: &'arena [f64] = self.arena.alloc_slice_fill_iter(
                        f32_data.iter().map(|&v| v as f64),
                    );

                    if let Err(e) = self.storage.vectors.insert_with_id::<fn(&crate::sparrow_engine::vector_core::vector::HVector, &heed3::RoTxn<'_>) -> bool>(
                        self.txn,
                        node.id,
                        vec_label,
                        f64_slice,
                        None,
                        self.arena,
                    ) {
                        result = Err(GraphError::VectorError(format!(
                            "Failed to insert vector for field '{vec_label}': {e:?}"
                        )));
                        break;
                    }
                }
            }
        }

        if result.is_ok() {
            result = Ok(TraversalValue::Node(node));
        }

        RwTraversalIterator {
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
            inner: std::iter::once(result),
        }
    }
}

