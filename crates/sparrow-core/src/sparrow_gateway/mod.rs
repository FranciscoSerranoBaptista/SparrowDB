#[cfg(feature = "lmdb")]
pub mod auth;
#[cfg(any(feature = "dev-instance", feature = "lmdb"))]
pub mod builtin;
pub mod embedding_providers;
pub mod gateway;
pub mod mem_monitor;
pub mod introspect_schema;
pub mod mcp;
pub mod router;
pub mod settings;
pub mod runtime_eval;
#[cfg(test)]
pub mod tests;
pub mod v1_compat;
pub mod worker_pool;
