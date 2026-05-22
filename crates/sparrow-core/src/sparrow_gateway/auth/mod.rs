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
    #[cfg(feature = "lmdb")]
    Io(std::io::Error),
    #[cfg(feature = "lmdb")]
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
            #[cfg(feature = "lmdb")]
            TokenError::Io(e) => write!(f, "io error: {e}"),
            #[cfg(feature = "lmdb")]
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

#[cfg(feature = "lmdb")]
impl From<std::io::Error> for TokenError {
    fn from(e: std::io::Error) -> Self {
        TokenError::Io(e)
    }
}

#[cfg(feature = "lmdb")]
impl From<serde_json::Error> for TokenError {
    fn from(e: serde_json::Error) -> Self {
        TokenError::Json(e)
    }
}
