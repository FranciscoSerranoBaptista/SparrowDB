#[cfg(feature = "lmdb")]
pub mod token_store;
#[cfg(feature = "lmdb")]
pub use token_store::TokenStore;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Admin,
    ReadWrite,
    ReadOnly,
}

impl Role {
    pub fn can_write(&self) -> bool {
        matches!(self, Role::Admin | Role::ReadWrite)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub id: String,       // 8-char hex (first 4 bytes of SHA-256 hash)
    pub name: String,     // human label
    pub role: Role,
    pub created_at: u64, // unix seconds
}

#[derive(Debug)]
pub enum TokenError {
    Unauthorized,
    InvalidKey,
    Forbidden,
    #[cfg(feature = "lmdb")]
    Storage(heed3::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Unauthorized => write!(f, "unauthorized"),
            TokenError::InvalidKey => write!(f, "invalid key"),
            TokenError::Forbidden => write!(f, "forbidden"),
            #[cfg(feature = "lmdb")]
            TokenError::Storage(e) => write!(f, "storage error: {e}"),
            TokenError::Io(e) => write!(f, "io error: {e}"),
            TokenError::Json(e) => write!(f, "json error: {e}"),
        }
    }
}

#[cfg(feature = "lmdb")]
impl From<heed3::Error> for TokenError {
    fn from(e: heed3::Error) -> Self {
        TokenError::Storage(e)
    }
}

impl From<std::io::Error> for TokenError {
    fn from(e: std::io::Error) -> Self {
        TokenError::Io(e)
    }
}

impl From<serde_json::Error> for TokenError {
    fn from(e: serde_json::Error) -> Self {
        TokenError::Json(e)
    }
}
