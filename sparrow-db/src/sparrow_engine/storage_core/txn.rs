// sparrow-db/src/sparrow_engine/storage_core/txn.rs
use crate::sparrow_engine::{traversal_core::{RTxn, WTxn}, types::GraphError};

pub trait ReadTransaction {
    fn read_txn(&self) -> Result<RTxn<'_>, GraphError>;
}

pub trait WriteTransaction {
    fn write_txn(&self) -> Result<WTxn<'_>, GraphError>;
}
