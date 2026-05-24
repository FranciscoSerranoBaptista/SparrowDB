use crate::{
    sparrow_engine::{
        traversal_core::{traversal_iter::RoTraversalIterator, traversal_value::TraversalValue},
        types::GraphError,
    },
    protocol::value::Value,
};
use itertools::Either;
use serde::Serialize;

use crate::{sparrow_engine::traversal_core::LMDB_STRING_HEADER_LENGTH, utils::items::Node};

pub trait NFromIndexAdapter<'db, 'arena, 'txn, 's, K: Into<Value> + Serialize>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    /// Returns a new iterator that will return the node from the secondary index.
    ///
    /// # Arguments
    ///
    /// * `index` - The name of the secondary index.
    /// * `key` - The key to search for in the secondary index.
    ///
    /// Note that both the `index` and `key` must be provided.
    /// The index must be a valid and existing secondary index and the key should match the type of the index.
    fn n_from_index(
        self,
        label: &'s str,
        index: &'s str,
        key: &'s K,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >
    where
        K: Into<Value> + Serialize + Clone;
}

impl<
    'db,
    'arena,
    'txn,
    's,
    K: Into<Value> + Serialize,
    I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
> NFromIndexAdapter<'db, 'arena, 'txn, 's, K> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    #[inline]
    fn n_from_index(
        self,
        label: &'s str,
        index: &'s str,
        key: &K,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >
    where
        K: Into<Value> + Serialize + Clone,
    {
        // Keys in the HashMap are "TypeName:field_name" since the
        // global-namespace fix.  Qualify by label so we find the right
        // per-type LMDB database.
        let qualified = format!("{label}:{index}");

        // Resolve the LMDB database for this index.  Any failure here is
        // surfaced as a GraphError iterator item rather than a panic so the
        // worker thread survives a misconfigured or stale `queries.rs`.
        let db = match self.storage.secondary_indices.get(qualified.as_str()) {
            Some(db) => db,
            None => {
                let err = GraphError::New(format!(
                    "Secondary Index '{index}' not found for type '{label}'"
                ));
                return RoTraversalIterator {
                    storage: self.storage,
                    arena: self.arena,
                    txn: self.txn,
                    inner: Either::Left(std::iter::once(Err(err))),
                };
            }
        };

        // Serialize the lookup key.
        let serialized_key = match bincode::serialize(&Value::from(key)) {
            Ok(bytes) => bytes,
            Err(e) => {
                return RoTraversalIterator {
                    storage: self.storage,
                    arena: self.arena,
                    txn: self.txn,
                    inner: Either::Left(std::iter::once(Err(GraphError::from(e)))),
                };
            }
        };

        // Open the prefix cursor.
        let prefix_iter = match db.0.prefix_iter(self.txn, &serialized_key) {
            Ok(iter) => iter,
            Err(e) => {
                return RoTraversalIterator {
                    storage: self.storage,
                    arena: self.arena,
                    txn: self.txn,
                    inner: Either::Left(std::iter::once(Err(GraphError::from(e)))),
                };
            }
        };

        let label_as_bytes = label.as_bytes();
        let res = Either::Right(prefix_iter.filter_map(move |item| {
            if let Ok((_, node_id)) = item &&
             let Some(value) = self.storage.nodes_db.get(self.txn, &node_id).ok()? {
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
                    match Node::<'arena>::from_bincode_bytes(node_id, value, self.arena) {
                        Ok(node) => {
                            return Some(Ok(TraversalValue::Node(node)));
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
        }));

        RoTraversalIterator {
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
            inner: res,
        }
    }
}

