use crate::{
    helix_engine::{
        traversal_core::{
            LMDB_STRING_HEADER_LENGTH, traversal_iter::RoTraversalIterator,
            traversal_value::TraversalValue,
        },
        types::GraphError,
    },
    utils::items::Node,
};

pub trait SearchBM25Adapter<'db, 'arena, 'txn>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    fn search_bm25<K>(
        self,
        label: &'arena str,
        query: &str,
        k: K,
    ) -> Result<
        RoTraversalIterator<
            'db,
            'arena,
            'txn,
            impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
        >,
        GraphError,
    >
    where
        K: TryInto<usize>,
        K::Error: std::fmt::Debug;
}

#[cfg(feature = "lmdb")]
impl<'db, 'arena, 'txn, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    SearchBM25Adapter<'db, 'arena, 'txn> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    fn search_bm25<K>(
        self,
        label: &'arena str,
        query: &str,
        k: K,
    ) -> Result<
        RoTraversalIterator<
            'db,
            'arena,
            'txn,
            impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
        >,
        GraphError,
    >
    where
        K: TryInto<usize>,
        K::Error: std::fmt::Debug,
    {
        use crate::helix_engine::bm25::BM25;

        let results = match self.storage.bm25.as_ref() {
            Some(s) => s.search(self.txn, query, k.try_into().unwrap(), self.arena)?,
            None => return Err(GraphError::from("BM25 not enabled!")),
        };

        let label_as_bytes = label.as_bytes();
        let iter = results.into_iter().filter_map(move |(id, score)| {
            if let Ok(Some(value)) = self.storage.nodes_db.get(self.txn, &id) {
            assert!(
                value.len() >= LMDB_STRING_HEADER_LENGTH,
                "value length does not contain header which means the `label` field was missing from the node on insertion"
            );
            let length_of_label_in_lmdb =
                u64::from_le_bytes(value[..LMDB_STRING_HEADER_LENGTH].try_into().unwrap()) as usize;

            if length_of_label_in_lmdb != label.len() {
                return None;
            }

            assert!(
                value.len() >= length_of_label_in_lmdb + LMDB_STRING_HEADER_LENGTH,
                "value length is not at least the header length plus the label length meaning there has been a corruption on node insertion"
            );
            let label_in_lmdb = &value[LMDB_STRING_HEADER_LENGTH
                ..LMDB_STRING_HEADER_LENGTH + length_of_label_in_lmdb];

            if label_in_lmdb == label_as_bytes {
                match Node::<'arena>::from_bincode_bytes(id, value, self.arena) {
                    Ok(node) => {
                        return Some(Ok(TraversalValue::NodeWithScore { node, score: score as f64 }));
                    }
                    Err(e) => {
                        println!("{} Error decoding node: {:?}", line!(), e);
                        return Some(Err(GraphError::ConversionError(e.to_string())));
                    }
                }
            } else {
                return None;
            }
            }
            None
        });

        Ok(RoTraversalIterator {
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
            inner: iter,
        })
    }
}

#[cfg(feature = "rocks")]
impl<'db, 'arena, 'txn, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    SearchBM25Adapter<'db, 'arena, 'txn> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    fn search_bm25<K>(
        self,
        label: &'arena str,
        query: &str,
        k: K,
    ) -> Result<
        RoTraversalIterator<
            'db,
            'arena,
            'txn,
            impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
        >,
        GraphError,
    >
    where
        K: TryInto<usize>,
        K::Error: std::fmt::Debug,
    {
        use crate::helix_engine::bm25::BM25;
        use crate::helix_engine::storage_core::HelixGraphStorage;

        let results = match self.storage.bm25.as_ref() {
            Some(s) => s.search(self.txn, query, k.try_into().unwrap())?,
            None => return Err(GraphError::from("BM25 not enabled!")),
        };

        let label_as_bytes = label.as_bytes();
        let label_len = label.len();
        let storage = self.storage;
        let arena = self.arena;
        let txn = self.txn;

        let iter = results.into_iter().filter_map(move |(id, score)| {
            let cf_nodes = storage.cf_nodes();
            let key = HelixGraphStorage::node_key(id);
            match txn.get_pinned_cf(&cf_nodes, key) {
                Ok(Some(value)) => {
                    if value.len() < LMDB_STRING_HEADER_LENGTH {
                        return None;
                    }
                    let length_of_label_in_db =
                        u64::from_le_bytes(value[..LMDB_STRING_HEADER_LENGTH].try_into().unwrap())
                            as usize;

                    if length_of_label_in_db != label_len {
                        return None;
                    }

                    let end = LMDB_STRING_HEADER_LENGTH + length_of_label_in_db;
                    if value.len() < end {
                        return None;
                    }

                    let label_in_db = &value[LMDB_STRING_HEADER_LENGTH..end];

                    if label_in_db == label_as_bytes {
                        match Node::<'arena>::from_bincode_bytes(id, &value, arena) {
                            Ok(node) => {
                                Some(Ok(TraversalValue::NodeWithScore { node, score: score as f64 }))
                            }
                            Err(e) => {
                                println!("{} Error decoding node: {:?}", line!(), e);
                                Some(Err(GraphError::ConversionError(e.to_string())))
                            }
                        }
                    } else {
                        None
                    }
                }
                Ok(None) => None,
                Err(e) => {
                    println!("{} Error getting node: {:?}", line!(), e);
                    None
                }
            }
        });

        Ok(RoTraversalIterator {
            storage,
            arena,
            txn,
            inner: iter,
        })
    }
}
