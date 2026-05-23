use crate::sparrow_engine::{
    storage_core::storage_methods::StorageMethods,
    traversal_core::{traversal_iter::RoTraversalIterator, traversal_value::TraversalValue},
    types::{GraphError, VectorError},
    vector_core::{HNSW, vector::HVector},
};
use std::iter::once;

pub trait SearchNAdapter<'db, 'arena, 'txn>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    fn search_n<F, K>(
        self,
        query: &'arena [f64],
        k: K,
        label: &'arena str,
        filter: Option<&'arena [F]>,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >
    where
        F: Fn(&HVector, &Txn) -> bool,
        K: TryInto<usize>,
        K::Error: std::fmt::Debug;
}

type Txn<'db> = heed3::RoTxn<'db>;

impl<'db, 'arena, 'txn, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    SearchNAdapter<'db, 'arena, 'txn> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    fn search_n<F, K>(
        self,
        query: &'arena [f64],
        k: K,
        label: &'arena str,
        filter: Option<&'arena [F]>,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >
    where
        F: Fn(&HVector, &Txn) -> bool,
        K: TryInto<usize>,
        K::Error: std::fmt::Debug,
    {
        let k_usize = match k.try_into() {
            Ok(n) => n,
            Err(_) => {
                let iter = once(Err(GraphError::New(
                    "vector search k must be a non-negative integer".to_string(),
                )))
                .collect::<Vec<_>>()
                .into_iter();
                return RoTraversalIterator {
                    storage: self.storage,
                    arena: self.arena,
                    txn: self.txn,
                    inner: iter,
                };
            }
        };

        let vectors = self.storage.vectors.search(
            self.txn,
            query,
            k_usize,
            label,
            filter,
            true,
            self.arena,
        );

        let iter = match vectors {
            Ok(vectors) => {
                let mut nodes: Vec<Result<TraversalValue, GraphError>> = Vec::new();
                for vector in vectors {
                    match self.storage.get_node(self.txn, vector.id, self.arena) {
                        Ok(node) => {
                            nodes.push(Ok(TraversalValue::Node(node)));
                        }
                        Err(GraphError::NodeNotFound) => {
                            // Node was soft-deleted from graph but vector entry still exists — skip
                        }
                        Err(e) => {
                            nodes.push(Err(e));
                        }
                    }
                }
                nodes.into_iter()
            }
            Err(VectorError::EntryPointNotFound) => {
                // Empty index — return no results (not an error)
                Vec::new().into_iter()
            }
            Err(VectorError::VectorNotFound(id)) => {
                let error = GraphError::VectorError(format!("vector not found for id {id}"));
                once(Err(error)).collect::<Vec<_>>().into_iter()
            }
            Err(VectorError::InvalidVectorData) => {
                let error = GraphError::VectorError("invalid vector data".to_string());
                once(Err(error)).collect::<Vec<_>>().into_iter()
            }
            Err(VectorError::InvalidVectorLength) => {
                let error = GraphError::VectorError("invalid vector dimensions!".to_string());
                once(Err(error)).collect::<Vec<_>>().into_iter()
            }
            Err(VectorError::ConversionError(e)) => {
                let error = GraphError::VectorError(format!("conversion error: {e}"));
                once(Err(error)).collect::<Vec<_>>().into_iter()
            }
            Err(VectorError::VectorCoreError(e)) => {
                let error = GraphError::VectorError(format!("vector core error: {e}"));
                once(Err(error)).collect::<Vec<_>>().into_iter()
            }
            Err(VectorError::VectorAlreadyDeleted(id)) => {
                let error = GraphError::VectorError(format!("vector already deleted for id {id}"));
                once(Err(error)).collect::<Vec<_>>().into_iter()
            }
            Err(VectorError::VectorDeleted) => {
                let error = GraphError::VectorError("vector was deleted".to_string());
                once(Err(error)).collect::<Vec<_>>().into_iter()
            }
            Err(VectorError::ZeroMagnitudeVector) => {
                let error = GraphError::VectorError(
                    "zero magnitude vector cannot be searched".to_string(),
                );
                once(Err(error)).collect::<Vec<_>>().into_iter()
            }
        };

        RoTraversalIterator {
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
            inner: iter,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bumpalo::Bump;
    use heed3::RoTxn;
    use tempfile::TempDir;

    use crate::{
        protocol::value::Value,
        sparrow_engine::{
            storage_core::SparrowGraphStorage,
            traversal_core::{
                ops::{
                    g::G,
                    source::add_n::AddNAdapter,
                    vectors::search_n::SearchNAdapter,
                },
                traversal_value::TraversalValue,
            },
            vector_core::vector::HVector,
        },
        utils::properties::ImmutablePropertiesMap,
    };

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

    #[test]
    fn test_add_n_and_search_n_round_trip() {
        let (_temp_dir, storage) = setup_test_db();

        // --- write phase: insert a node with a vector embedding ---
        let write_arena = Bump::new();
        let mut txn = storage.graph_env.write_txn().unwrap();

        let props_map = ImmutablePropertiesMap::new(
            1,
            std::iter::once((write_arena.alloc_str("name") as &str, Value::String("Alice".to_string()))),
            &write_arena,
        );

        let inserted = G::new_mut(&storage, &write_arena, &mut txn)
            .add_n_with_vectors(
                "Person",
                Some(props_map),
                None,
                Some(&[("Person.embedding", &[0.1_f32, 0.2, 0.3, 0.4])]),
            )
            .collect_to_obj()
            .unwrap();

        let inserted_id = inserted.id();
        let inserted_label = inserted.label().to_string();

        txn.commit().unwrap();

        // --- read phase: search for the node by its vector ---
        let read_arena = Bump::new();
        let txn = storage.graph_env.read_txn().unwrap();

        let query = read_arena.alloc_slice_copy(&[0.1_f64, 0.2, 0.3, 0.4]);
        let label = read_arena.alloc_str("Person.embedding");

        let results = G::new(&storage, &txn, &read_arena)
            .search_n::<Filter, usize>(query, 1usize, label, None)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        // --- assertions ---
        assert_eq!(results.len(), 1, "expected exactly one result from search_n");

        let result = &results[0];
        assert_eq!(result.id(), inserted_id, "result node id should match inserted node id");
        assert_eq!(inserted_label, "Person", "inserted node should have label 'Person'");
        assert_eq!(result.label(), "Person", "result node should have label 'Person'");

        if let TraversalValue::Node(node) = result {
            let name = node.get_property("name");
            assert_eq!(
                name,
                Some(&Value::String("Alice".to_string())),
                "node should have name='Alice'"
            );
        } else {
            panic!("expected TraversalValue::Node, got {:?}", result);
        }
    }
}
