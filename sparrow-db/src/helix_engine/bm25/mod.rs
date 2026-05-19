#[cfg(feature = "lmdb")]
pub mod lmdb_bm25;
#[cfg(feature = "rocks")]
pub mod rocks_bm25;

#[cfg(feature = "lmdb")]
pub use lmdb_bm25::{
    BM25, BM25Flatten, BM25Metadata, HBM25Config, HybridSearch, METADATA_KEY,
    BM25_SCHEMA_VERSION, BM25_SCHEMA_VERSION_KEY, PostingListEntry, ReversePostingEntry,
    build_bm25_payload,
};
#[cfg(feature = "rocks")]
pub use rocks_bm25::{BM25, BM25Flatten, BM25Metadata, HBM25Config, HybridSearch, METADATA_KEY};

#[cfg(test)]
pub mod bm25_tests;
