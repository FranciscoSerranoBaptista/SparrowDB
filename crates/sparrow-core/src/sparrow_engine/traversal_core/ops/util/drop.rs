use crate::sparrow_engine::{
    bm25::BM25,
    storage_core::storage_methods::StorageMethods,
    storage_core::SparrowGraphStorage,
    traversal_core::{traversal_value::TraversalValue, WTxn},
    types::GraphError,
};
use std::sync::atomic::Ordering;

pub struct Drop<I> {
    pub iter: I,
}

impl<'db, 'arena, 'txn, I> Drop<I>
where
    I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
{
    pub fn drop_traversal(
        iter: I,
        storage: &'db SparrowGraphStorage,
        txn: &'txn mut WTxn<'db>,
    ) -> Result<(), GraphError> {
        iter.into_iter().filter_map(|item| item.ok()).try_for_each(
            |item| -> Result<(), GraphError> {
                match item {
                    TraversalValue::Node(node) => match storage.drop_node(txn, node.id) {
                        Ok(_) => {
                            // BM25 delete must succeed: if it fails, the node record is gone
                            // but its BM25 document would survive, producing ghost search results.
                            // Return an error so the transaction can be rolled back and the
                            // caller retries or surfaces the failure.
                            if let Some(bm25) = storage.bm25.as_ref().filter(|_| {
                                !storage.skip_bm25_writes.load(Ordering::Acquire)
                                    && !storage.bm25_exclude_labels.contains(node.label)
                            }) {
                                bm25.delete_doc(txn, node.id)?;
                            }
                            tracing::debug!(node_id = ?node.id, "drop: node removed");
                            Ok(())
                        }
                        Err(e) => Err(e),
                    },
                    TraversalValue::Edge(edge) => match storage.drop_edge(txn, edge.id) {
                        Ok(_) => Ok(()),
                        Err(e) => Err(e),
                    },
                    TraversalValue::Vector(vector) => match storage.drop_vector(txn, vector.id) {
                        Ok(_) => Ok(()),
                        Err(e) => Err(e),
                    },
                    TraversalValue::VectorNodeWithoutVectorData(vector) => {
                        match storage.drop_vector(txn, vector.id) {
                            Ok(_) => Ok(()),
                            Err(e) => Err(e),
                        }
                    }
                    TraversalValue::Empty => Ok(()),
                    _ => Err(GraphError::ConversionError(format!(
                        "Incorrect Type: {item:?}"
                    ))),
                }
            },
        )
    }
}
