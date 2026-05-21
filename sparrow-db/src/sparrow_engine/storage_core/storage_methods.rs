use crate::sparrow_engine::types::GraphError;
use crate::utils::items::{Edge, Node};
use heed3::{RoTxn, RwTxn};

pub trait DBMethods {
    fn create_secondary_index(&mut self, name: &str) -> Result<(), GraphError>;
    fn drop_secondary_index(&mut self, name: &str) -> Result<(), GraphError>;
}

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
