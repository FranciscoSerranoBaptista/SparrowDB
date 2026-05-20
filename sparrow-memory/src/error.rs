use sparrow_db::sparrow_engine::types::GraphError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("storage error: {0}")]
    Storage(#[from] GraphError),
    #[error("serialization error: {0}")]
    Serialization(bincode::Error),
    #[error("index not found: {0}")]
    IndexNotFound(String),
    #[error("node not found: {0}")]
    NodeNotFound(u128),
    #[error("heed error: {0}")]
    Heed(heed3::Error),
}
