use sparrow_db::{
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, version_info::VersionInfo},
        traversal_core::config::{Config, GraphConfig},
        types::SecondaryIndex,
    },
};
use std::sync::Arc;
use crate::{error::MemoryError, indices::ALL_INDICES, thread::ThreadHandle, types::ThreadSummary};
use sparrow_db::protocol::value::Value;
use sparrow_db::sparrow_engine::storage_core::storage_methods::StorageMethods;

pub struct MemoryConfig {
    pub path: String,
    pub db_max_size_gb: Option<usize>,
    pub embedding_model: Option<String>,
}

pub struct MemoryStore {
    pub(crate) storage: Arc<SparrowGraphStorage>,
}

impl MemoryStore {
    pub fn open(config: MemoryConfig) -> Result<Self, MemoryError> {
        std::fs::create_dir_all(&config.path)
            .map_err(|e| MemoryError::Storage(sparrow_db::sparrow_engine::types::GraphError::Io(e)))?;

        let secondary_indices: Vec<SecondaryIndex> = ALL_INDICES
            .iter()
            .map(|name| SecondaryIndex::Index(name.to_string()))
            .collect();

        let sparrow_config = Config {
            graph_config: Some(GraphConfig {
                secondary_indices: Some(secondary_indices),
            }),
            db_max_size_gb: config.db_max_size_gb,
            ..Config::default()
        };

        let storage = SparrowGraphStorage::new(&config.path, sparrow_config, VersionInfo::default())
            .map_err(MemoryError::Storage)?;

        Ok(Self { storage: Arc::new(storage) })
    }

    /// Number of registered secondary indices — used for test assertions.
    pub fn index_names(&self) -> Vec<String> {
        self.storage.secondary_indices.keys().cloned().collect()
    }

    /// Get or create a research thread for `agent` with the given `name`.
    pub fn thread(&self, agent: &str, name: &str, goal: &str) -> Result<ThreadHandle, MemoryError> {
        ThreadHandle::get_or_create(Arc::clone(&self.storage), agent, name, goal)
    }

    /// List all threads for an agent.
    pub fn threads(&self, agent: &str) -> Result<Vec<ThreadSummary>, MemoryError> {
        let rtxn = self.storage.graph_env.read_txn().map_err(MemoryError::Heed)?;
        let arena = bumpalo::Bump::new();
        let mut result = Vec::new();
        for item in self.storage.nodes_db.iter(&rtxn).map_err(MemoryError::Heed)? {
            let (node_id, _) = item.map_err(MemoryError::Heed)?;
            let node = match self.storage.get_node(&rtxn, node_id, &arena) {
                Ok(n) => n,
                Err(_) => continue,
            };
            if node.label != "research_thread" {
                continue;
            }
            let agent_match = node
                .get_property("agent_name")
                .map(|v| matches!(v, Value::String(s) if s.as_str() == agent))
                .unwrap_or(false);
            if !agent_match {
                continue;
            }
            result.push(ThreadSummary {
                id: crate::types::ThreadId(node_id),
                name: node
                    .get_property("name")
                    .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                    .unwrap_or_default(),
                goal: node
                    .get_property("goal")
                    .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                    .unwrap_or_default(),
                status: node
                    .get_property("status")
                    .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                    .unwrap_or_default(),
            });
        }
        Ok(result)
    }
}
