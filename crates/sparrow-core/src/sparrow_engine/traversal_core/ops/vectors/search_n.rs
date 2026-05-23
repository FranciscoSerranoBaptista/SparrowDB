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
            Err(id) => {
                let error = GraphError::VectorError(format!("vector already deleted for id {id}"));
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
