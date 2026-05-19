use crate::{
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, storage_methods::StorageMethods},
        traversal_core::{traversal_iter::RoTraversalIterator, traversal_value::TraversalValue},
        types::GraphError,
    },
    utils::label_hash::hash_label,
};

pub trait OutEdgesAdapter<'db, 'arena, 'txn, 's>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    /// Returns an iterator containing the edges that have an outgoing edge with the given label.
    ///
    /// Note that the `edge_label` cannot be empty and must be a valid, existing edge label.
    ///
    /// To provide safety, you cannot get all outgoing edges as it would be ambiguous as to what
    /// type that resulting edge would be.
    fn out_e(
        self,
        edge_label: &'s str,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >;
}

#[cfg(feature = "lmdb")]
impl<'db, 'arena, 'txn, 's, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    OutEdgesAdapter<'db, 'arena, 'txn, 's> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    #[inline]
    fn out_e(
        self,
        edge_label: &'s str,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        // iterate through the iterator and create a new iterator on the out edges
        let iter = self
            .inner
            .filter_map(move |item| {
                let edge_label_hash = hash_label(edge_label, None);

                let prefix = SparrowGraphStorage::out_edge_key(
                    &match item {
                        Ok(item) => item.id(),
                        Err(_) => return None,
                    },
                    &edge_label_hash,
                );
                match self
                    .storage
                    .out_edges_db
                    .lazily_decode_data()
                    .get_duplicates(self.txn, &prefix)
                {
                    Ok(Some(iter)) => {
                        let iter = iter.map(|item| match item {
                            Ok((_, data)) => match data.decode() {
                                Ok(data) => {
                                    let (edge_id, _) =
                                        match SparrowGraphStorage::unpack_adj_edge_data(data) {
                                            Ok(data) => data,
                                            Err(e) => return Err(e),
                                        };
                                    match self.storage.get_edge(self.txn, edge_id, self.arena) {
                                        Ok(edge) => Ok(TraversalValue::Edge(edge)),
                                        Err(e) => Err(e),
                                    }
                                }
                                Err(e) => Err(GraphError::DecodeError(e.to_string())),
                            },
                            Err(e) => Err(e.into()),
                        });
                        Some(iter)
                    }
                    Ok(None) => None,
                    Err(e) => {
                        println!("Error getting in edges: {e:?}");
                        None
                    }
                }
            })
            .flatten();
        RoTraversalIterator {
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
            inner: iter,
        }
    }
}

#[cfg(feature = "rocks")]
impl<'db, 'arena, 'txn, 's, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    OutEdgesAdapter<'db, 'arena, 'txn, 's> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    #[inline]
    fn out_e(
        self,
        edge_label: &'s str,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        use crate::sparrow_engine::rocks_utils::RocksUtils;

        let storage = self.storage;
        let arena = self.arena;
        let txn = self.txn;

        let iter = self
            .inner
            .filter_map(move |item| {
                let edge_label_hash = hash_label(edge_label, None);
                let node_id = match item {
                    Ok(item) => item.id(),
                    Err(_) => return None,
                };
                let prefix = SparrowGraphStorage::out_edge_key_prefix(node_id, &edge_label_hash);
                let cf_out_edges = storage.cf_out_edges();

                let mut raw_iter = txn.raw_prefix_iter(&cf_out_edges, &prefix);
                let mut results: Vec<Result<TraversalValue<'arena>, GraphError>> = Vec::new();

                while let Some(key) = raw_iter.key() {
                    if !key.starts_with(&prefix) {
                        break;
                    }
                    // Key: from_node(16) | label(4) | to_node(16) | edge_id(16)
                    match SparrowGraphStorage::unpack_adj_edge_key(key) {
                        Ok((_, _, _, edge_id)) => {
                            match storage.get_edge(txn, edge_id, arena) {
                                Ok(edge) => {
                                    results.push(Ok(TraversalValue::Edge(edge)));
                                }
                                Err(e) => {
                                    results.push(Err(e));
                                }
                            }
                        }
                        Err(e) => {
                            results.push(Err(e));
                        }
                    }
                    raw_iter.next();
                }

                if results.is_empty() {
                    None
                } else {
                    Some(results.into_iter())
                }
            })
            .flatten();

        RoTraversalIterator {
            storage,
            arena,
            txn,
            inner: iter,
        }
    }
}
