use crate::errors::ConfigError;
use sparrow_db::sparrow_engine::{
    traversal_core::config::{
        Config as RuntimeConfig, GraphConfig as EngineGraphConfig,
        VectorConfig as EngineVectorConfig,
    },
    types::SecondaryIndex,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparrowConfig {
    pub project: ProjectConfig,
    #[serde(default)]
    pub local: HashMap<String, LocalInstanceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(
        default = "default_queries_path",
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub queries: PathBuf,
    #[serde(default = "default_container_runtime")]
    pub container_runtime: ContainerRuntime,
}

fn default_queries_path() -> PathBuf {
    PathBuf::from("./db/")
}

fn serialize_path<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&path.to_string_lossy())
}

fn deserialize_path<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(PathBuf::from(s.replace('\\', "/")))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContainerRuntime {
    #[default]
    Docker,
    Podman,
    OrbStack,
}

impl ContainerRuntime {
    pub fn binary(&self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::OrbStack => "docker", // OrbStack provides its own docker-compatible binary
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Docker => "DOCKER",
            Self::Podman => "PODMAN",
            Self::OrbStack => "ORBSTACK",
        }
    }
}

fn default_container_runtime() -> ContainerRuntime {
    ContainerRuntime::Docker
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorConfig {
    #[serde(default = "default_m")]
    pub m: u32,
    #[serde(default = "default_ef_construction")]
    pub ef_construction: u32,
    #[serde(default = "default_ef_search")]
    pub ef_search: u32,
    #[serde(default = "default_db_max_size_gb")]
    pub db_max_size_gb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GraphConfig {
    #[serde(default)]
    pub secondary_indices: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConfig {
    #[serde(default, skip_serializing_if = "is_default_vector_config")]
    pub vector_config: VectorConfig,
    #[serde(default, skip_serializing_if = "is_default_graph_config")]
    pub graph_config: GraphConfig,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub mcp: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub bm25: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(
        default = "default_embedding_model",
        skip_serializing_if = "is_default_embedding_model"
    )]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphvis_node_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalInstanceConfig {
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default = "default_dev_build_mode")]
    pub build_mode: BuildMode,
    #[serde(default)]
    pub storage_backend: StorageBackend,
    #[serde(flatten)]
    pub db_config: DbConfig,
}

impl DbConfig {
    #[allow(dead_code)]
    pub fn to_runtime_config(&self) -> RuntimeConfig {
        let secondary_indices = if self.graph_config.secondary_indices.is_empty() {
            None
        } else {
            Some(
                self.graph_config
                    .secondary_indices
                    .iter()
                    .cloned()
                    .map(SecondaryIndex::Index)
                    .collect(),
            )
        };

        RuntimeConfig {
            vector_config: Some(EngineVectorConfig {
                m: Some(self.vector_config.m as usize),
                ef_construction: Some(self.vector_config.ef_construction as usize),
                ef_search: Some(self.vector_config.ef_search as usize),
            }),
            graph_config: Some(EngineGraphConfig { secondary_indices }),
            db_max_size_gb: Some(self.vector_config.db_max_size_gb as usize),
            mcp: Some(self.mcp),
            bm25: Some(self.bm25),
            schema: self.schema.clone(),
            embedding_model: self.embedding_model.clone(),
            graphvis_node_label: self.graphvis_node_label.clone(),
            hql_schema_raw: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BuildMode {
    #[default]
    Dev,
    Release,
    Debug,
}

pub fn default_dev_build_mode() -> BuildMode {
    BuildMode::Dev
}

#[allow(dead_code)]
pub fn default_release_build_mode() -> BuildMode {
    BuildMode::Release
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    #[default]
    Lmdb,
    Rocks,
}

fn default_true() -> bool {
    true
}

fn default_m() -> u32 { 16 }
fn default_ef_construction() -> u32 { 128 }
fn default_ef_search() -> u32 { 768 }
fn default_db_max_size_gb() -> u32 { 4 }

fn default_embedding_model() -> Option<String> {
    Some("text-embedding-ada-002".to_string())
}

fn is_default_embedding_model(value: &Option<String>) -> bool {
    *value == default_embedding_model()
}

fn is_true(value: &bool) -> bool { *value }

fn is_default_vector_config(value: &VectorConfig) -> bool {
    *value == VectorConfig::default()
}

fn is_default_graph_config(value: &GraphConfig) -> bool {
    *value == GraphConfig::default()
}

impl Default for VectorConfig {
    fn default() -> Self {
        VectorConfig {
            m: default_m(),
            ef_construction: default_ef_construction(),
            ef_search: default_ef_search(),
            db_max_size_gb: default_db_max_size_gb(),
        }
    }
}

impl Default for DbConfig {
    fn default() -> Self {
        DbConfig {
            vector_config: VectorConfig::default(),
            graph_config: GraphConfig::default(),
            mcp: true,
            bm25: true,
            schema: None,
            embedding_model: default_embedding_model(),
            graphvis_node_label: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum InstanceInfo<'a> {
    Local(&'a LocalInstanceConfig),
}

impl<'a> InstanceInfo<'a> {
    pub fn build_mode(&self) -> BuildMode {
        match self {
            InstanceInfo::Local(LocalInstanceConfig { build_mode, .. }) => *build_mode,
        }
    }

    pub fn storage_backend(&self) -> StorageBackend {
        match self {
            InstanceInfo::Local(LocalInstanceConfig { storage_backend, .. }) => *storage_backend,
        }
    }

    pub fn port(&self) -> Option<u16> {
        match self {
            InstanceInfo::Local(config) => config.port,
        }
    }

    pub fn db_config(&self) -> &DbConfig {
        match self {
            InstanceInfo::Local(LocalInstanceConfig { db_config, .. }) => db_config,
        }
    }

    #[allow(dead_code)]
    pub fn is_local(&self) -> bool {
        true
    }

    pub fn should_build_docker_image(&self) -> bool {
        true
    }

    pub fn docker_build_target(&self) -> Option<&str> {
        None
    }

    pub fn to_legacy_json(&self) -> serde_json::Value {
        let db_config = self.db_config();

        let mut json = serde_json::json!({
            "vector_config": {
                "m": db_config.vector_config.m,
                "ef_construction": db_config.vector_config.ef_construction,
                "ef_search": db_config.vector_config.ef_search,
                "db_max_size": db_config.vector_config.db_max_size_gb
            },
            "graph_config": {
                "secondary_indices": db_config.graph_config.secondary_indices
            },
            "db_max_size_gb": db_config.vector_config.db_max_size_gb,
            "mcp": db_config.mcp,
            "bm25": db_config.bm25
        });

        if let Some(schema) = &db_config.schema {
            json["schema"] = serde_json::Value::String(schema.clone());
        }
        if let Some(embedding_model) = &db_config.embedding_model {
            json["embedding_model"] = serde_json::Value::String(embedding_model.clone());
        }
        if let Some(graphvis_node_label) = &db_config.graphvis_node_label {
            json["graphvis_node_label"] = serde_json::Value::String(graphvis_node_label.clone());
        }

        json
    }
}

impl SparrowConfig {
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path).map_err(|source| ConfigError::ReadSparrowConfig {
            path: path.to_path_buf(),
            source,
        })?;

        let config: SparrowConfig =
            toml::from_str(&content).map_err(|source| ConfigError::ParseSparrowConfig {
                path: path.to_path_buf(),
                source,
            })?;

        config.validate(path)?;
        Ok(config)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), ConfigError> {
        let content = toml::to_string_pretty(self)
            .map_err(|source| ConfigError::SerializeSparrowConfig { source })?;

        fs::write(path, content).map_err(|source| ConfigError::WriteSparrowConfig {
            path: path.to_path_buf(),
            source,
        })?;

        Ok(())
    }

    fn validate(&self, path: &Path) -> Result<(), ConfigError> {
        let relative_path = std::env::current_dir()
            .ok()
            .and_then(|cwd| path.strip_prefix(&cwd).ok())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf());

        if self.project.name.is_empty() {
            return Err(ConfigError::EmptyProjectName { path: relative_path.clone() });
        }

        if self.local.is_empty() {
            return Err(ConfigError::MissingInstances { path: relative_path.clone() });
        }

        for (name, config) in &self.local {
            if name.is_empty() {
                return Err(ConfigError::EmptyInstanceName { path: relative_path.clone() });
            }
            if config.build_mode == BuildMode::Debug {
                return Err(ConfigError::DeprecatedBuildMode { path: relative_path.clone() });
            }
        }

        Ok(())
    }

    pub fn get_instance(&self, name: &str) -> Result<InstanceInfo<'_>, ConfigError> {
        if let Some(local_config) = self.local.get(name) {
            return Ok(InstanceInfo::Local(local_config));
        }

        Err(ConfigError::InstanceNotFound { name: name.to_string() })
    }

    pub fn list_instances(&self) -> Vec<&String> {
        self.local.keys().collect()
    }

    pub fn list_instances_with_types(&self) -> Vec<(&String, &'static str)> {
        let mut instances: Vec<_> = self.local.keys().map(|name| (name, "local")).collect();
        instances.sort_by(|a, b| a.0.cmp(b.0));
        instances
    }

    pub fn default_config(project_name: &str) -> Self {
        let mut local = HashMap::new();
        local.insert(
            "dev".to_string(),
            LocalInstanceConfig {
                port: Some(6969),
                build_mode: BuildMode::Dev,
                storage_backend: StorageBackend::Lmdb,
                db_config: DbConfig::default(),
            },
        );

        SparrowConfig {
            project: ProjectConfig {
                id: None,
                name: project_name.to_string(),
                queries: default_queries_path(),
                container_runtime: default_container_runtime(),
            },
            local,
        }
    }
}
