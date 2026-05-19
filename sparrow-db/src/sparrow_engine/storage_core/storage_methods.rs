use crate::sparrow_engine::types::GraphError;
use crate::utils::items::{Edge, Node};

// NOTE: DBMethods is implemented for lmdb only; rocks impl is added in Task 8
pub trait DBMethods {
    fn create_secondary_index(&mut self, name: &str) -> Result<(), GraphError>;
    fn drop_secondary_index(&mut self, name: &str) -> Result<(), GraphError>;
}

#[cfg(feature = "lmdb")]
use heed3::{RoTxn, RwTxn};

#[cfg(feature = "lmdb")]
pub trait StorageMethods {
    fn get_node<'arena>(
        &self,
        txn: &RoTxn,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Node<'arena>, GraphError>;

    fn get_edge<'arena>(
        &self,
        txn: &RoTxn,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Edge<'arena>, GraphError>;

    fn drop_node(&self, txn: &mut RwTxn, id: u128) -> Result<(), GraphError>;
    fn drop_edge(&self, txn: &mut RwTxn, id: u128) -> Result<(), GraphError>;
    fn drop_vector(&self, txn: &mut RwTxn, id: u128) -> Result<(), GraphError>;
}

#[cfg(feature = "rocks")]
pub trait StorageMethods {
    fn get_node<'arena>(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Node<'arena>, GraphError>;

    fn get_edge<'arena>(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Edge<'arena>, GraphError>;

    fn drop_node(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
    ) -> Result<(), GraphError>;

    fn drop_edge(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
    ) -> Result<(), GraphError>;

    fn drop_vector(
        &self,
        txn: &rocksdb::Transaction<'_, rocksdb::TransactionDB>,
        id: u128,
    ) -> Result<(), GraphError>;
}
