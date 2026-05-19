use crate::{
    helix_engine::{
        storage_core::{HelixGraphStorage, storage_methods::StorageMethods},
        traversal_core::{traversal_iter::RoTraversalIterator, traversal_value::TraversalValue},
        types::GraphError,
    },
    utils::label_hash::hash_label,
};

pub trait OutAdapter<'db, 'arena, 'txn, 's>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    /// Returns an iterator containing the nodes that have an outgoing edge with the given label.
    ///
    /// Note that the `edge_label` cannot be empty and must be a valid, existing edge label.
    ///
    /// To provide safety, you cannot get all outgoing nodes as it would be ambiguous as to what
    /// type that resulting node would be.
    fn out_vec(
        self,
        edge_label: &'s str,
        get_vector_data: bool,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >;

    fn out_node(
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
    OutAdapter<'db, 'arena, 'txn, 's> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    #[inline]
    fn out_vec(
        self,
        edge_label: &'s str,
        get_vector_data: bool,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        let iter = self
            .inner
            .filter_map(move |item| {
                let edge_label_hash = hash_label(edge_label, None);
                let prefix = HelixGraphStorage::out_edge_key(
                    &match item {
                        Ok(item) => item.id(),
                        Err(_) => return None,
                    },
                    &edge_label_hash,
                );

                match self.storage.out_edges_db.get_duplicates(self.txn, &prefix) {
                    Ok(Some(iter)) => Some(iter.filter_map(move |item| {
                        if let Ok((_, value)) = item {
                            let (_, item_id) = match HelixGraphStorage::unpack_adj_edge_data(value)
                            {
                                Ok(data) => data,
                                Err(e) => {
                                    println!("Error unpacking edge data: {e:?}");
                                    return Some(Err(e));
                                }
                            };
                            if get_vector_data {
                                if let Ok(vec) = self
                                    .storage
                                    .vectors
                                    .get_full_vector(self.txn, item_id, self.arena)
                                {
                                    return Some(Ok(TraversalValue::Vector(vec)));
                                }
                            } else if let Ok(Some(vec)) = self
                                .storage
                                .vectors
                                .get_vector_properties(self.txn, item_id, self.arena)
                            {
                                return Some(Ok(TraversalValue::VectorNodeWithoutVectorData(vec)));
                            }
                            None
                        } else {
                            None
                        }
                    })),
                    Ok(None) => None,
                    Err(e) => {
                        println!("{} Error getting out edges: {:?}", line!(), e);
                        None
                    }
                }
            })
            .flatten();

        RoTraversalIterator {
            inner: iter,
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
        }
    }

    #[inline]
    fn out_node(
        self,
        edge_label: &'s str,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        let iter = self
            .inner
            .filter_map(move |item| {
                let edge_label_hash = hash_label(edge_label, None);
                let prefix = HelixGraphStorage::out_edge_key(
                    &match item {
                        Ok(item) => item.id(),
                        Err(_) => return None,
                    },
                    &edge_label_hash,
                );
                match self.storage.out_edges_db.get_duplicates(self.txn, &prefix) {
                    Ok(Some(iter)) => Some(iter.filter_map(move |item| {
                        if let Ok((_, data)) = item {
                            let (_, item_id) = match HelixGraphStorage::unpack_adj_edge_data(data) {
                                Ok(data) => data,
                                Err(e) => {
                                    println!("Error unpacking edge data: {e:?}");
                                    return Some(Err(e));
                                }
                            };
                            if let Ok(node) = self.storage.get_node(self.txn, item_id, self.arena)
                            {
                                return Some(Ok(TraversalValue::Node(node)));
                            }
                        }
                        None
                    })),
                    Ok(None) => None,
                    Err(e) => {
                        println!("{} Error getting out nodes: {:?}", line!(), e);
                        None
                    }
                }
            })
            .flatten();

        RoTraversalIterator {
            inner: iter,
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
        }
    }
}

#[cfg(feature = "rocks")]
impl<'db, 'arena, 'txn, 's, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    OutAdapter<'db, 'arena, 'txn, 's> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    #[inline]
    fn out_vec(
        self,
        edge_label: &'s str,
        get_vector_data: bool,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        use crate::helix_engine::rocks_utils::RocksUtils;

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
                let prefix = HelixGraphStorage::out_edge_key_prefix(node_id, &edge_label_hash);
                let cf_out_edges = storage.cf_out_edges();

                let mut raw_iter = txn.raw_prefix_iter(&cf_out_edges, &prefix);
                let mut results: Vec<Result<TraversalValue<'arena>, GraphError>> = Vec::new();

                while let Some(key) = raw_iter.key() {
                    if !key.starts_with(&prefix) {
                        break;
                    }
                    // Key: from_node(16) | label(4) | to_node(16) | edge_id(16)
                    match HelixGraphStorage::unpack_adj_edge_key(key) {
                        Ok((_, _, to_node_id, _)) => {
                            if get_vector_data {
                                if let Ok(vec) =
                                    storage.vectors.get_full_vector(txn, to_node_id, arena)
                                {
                                    results.push(Ok(TraversalValue::Vector(vec)));
                                }
                            } else if let Ok(Some(vec)) =
                                storage.vectors.get_vector_properties(txn, to_node_id, arena)
                            {
                                results.push(Ok(TraversalValue::VectorNodeWithoutVectorData(vec)));
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
            inner: iter,
            storage,
            arena,
            txn,
        }
    }

    #[inline]
    fn out_node(
        self,
        edge_label: &'s str,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        use crate::helix_engine::rocks_utils::RocksUtils;

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
                let prefix = HelixGraphStorage::out_edge_key_prefix(node_id, &edge_label_hash);
                let cf_out_edges = storage.cf_out_edges();

                let mut raw_iter = txn.raw_prefix_iter(&cf_out_edges, &prefix);
                let mut results: Vec<Result<TraversalValue<'arena>, GraphError>> = Vec::new();

                while let Some(key) = raw_iter.key() {
                    if !key.starts_with(&prefix) {
                        break;
                    }
                    match HelixGraphStorage::unpack_adj_edge_key(key) {
                        Ok((_, _, to_node_id, _)) => {
                            match storage.get_node(txn, to_node_id, arena) {
                                Ok(node) => {
                                    results.push(Ok(TraversalValue::Node(node)));
                                }
                                Err(e) => {
                                    println!("Error getting node: {e:?}");
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
            inner: iter,
            storage,
            arena,
            txn,
        }
    }
}
