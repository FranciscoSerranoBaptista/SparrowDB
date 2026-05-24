use crate::{
    sparrow_engine::types::{GraphError, SecondaryIndex},
    sparrowc::analyzer::IntrospectionData,
};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VectorConfig {
    pub m: Option<usize>,
    pub ef_construction: Option<usize>,
    pub ef_search: Option<usize>,
}

impl Default for VectorConfig {
    fn default() -> Self {
        Self {
            m: Some(16),
            ef_construction: Some(128),
            ef_search: Some(768),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct GraphConfig {
    pub secondary_indices: Option<Vec<SecondaryIndex>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub vector_config: Option<VectorConfig>,
    pub graph_config: Option<GraphConfig>,
    pub db_max_size_gb: Option<usize>,
    pub mcp: Option<bool>,
    pub bm25: Option<bool>,
    pub schema: Option<String>,
    pub embedding_model: Option<String>,
    pub graphvis_node_label: Option<String>,
    #[serde(skip)]
    pub hql_schema_raw: Option<String>,
}

/// Compute the minimum number of `#` guards needed for a Rust raw string literal
/// that contains `s`. Returns at least 1.
fn required_raw_str_hashes(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut max_hashes = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut count = 0;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'#' {
                count += 1;
                j += 1;
            }
            if count > max_hashes {
                max_hashes = count;
            }
        }
        i += 1;
    }
    max_hashes + 1
}

impl Config {
    pub fn new(
        m: usize,
        ef_construction: usize,
        ef_search: usize,
        db_max_size_gb: usize,
        mcp: bool,
        bm25: bool,
        schema: Option<String>,
        embedding_model: Option<String>,
        graphvis_node_label: Option<String>,
        hql_schema_raw: Option<String>,
    ) -> Self {
        Self {
            vector_config: Some(VectorConfig {
                m: Some(m),
                ef_construction: Some(ef_construction),
                ef_search: Some(ef_search),
            }),
            graph_config: Some(GraphConfig {
                secondary_indices: None,
            }),
            db_max_size_gb: Some(db_max_size_gb),
            mcp: Some(mcp),
            bm25: Some(bm25),
            schema,
            embedding_model,
            graphvis_node_label,
            hql_schema_raw,
        }
    }

    pub fn from_files(config_path: PathBuf, schema_path: PathBuf) -> Result<Self, GraphError> {
        if !config_path.exists() {
            println!("no config path!");
            return Err(GraphError::ConfigFileNotFound);
        }

        let config = std::fs::read_to_string(config_path)?;
        let mut config = sonic_rs::from_str::<Config>(&config)?;

        if schema_path.exists() {
            let schema_string = std::fs::read_to_string(schema_path)?;
            config.schema = Some(schema_string);
        } else {
            config.schema = None;
        }

        Ok(config)
    }

    pub fn from_file(config_path: PathBuf) -> Result<Self, GraphError> {
        if !config_path.exists() {
            println!("no config path!");
            return Err(GraphError::ConfigFileNotFound);
        }

        let config = std::fs::read_to_string(config_path)?;
        let mut config = sonic_rs::from_str::<Config>(&config)?;

        // Schema will be populated from INTROSPECTION_DATA during code generation
        config.schema = None;

        Ok(config)
    }

    pub fn init_config() -> String {
        r#"
{
	"vector_config": {
		"m": 16,
		"ef_construction": 128,
		"ef_search": 768
	},
	"graph_config": {
		"secondary_indices": []
	},
	"db_max_size_gb": 4,
	"mcp": true,
	"bm25": true,
	"embedding_model": "text-embedding-ada-002",
	"graphvis_node_label": ""
}
        "#
        .trim()
        .to_string()
    }

    pub fn to_json(&self) -> String {
        sonic_rs::to_string_pretty(self).unwrap()
    }

    pub fn get_vector_config(&self) -> VectorConfig {
        self.vector_config.clone().unwrap_or_default()
    }

    pub fn get_graph_config(&self) -> GraphConfig {
        self.graph_config.clone().unwrap_or_default()
    }

    pub fn get_db_max_size_gb(&self) -> usize {
        self.db_max_size_gb.unwrap_or(4)
    }

    pub fn get_mcp(&self) -> bool {
        self.mcp.unwrap_or(true)
    }

    pub fn get_bm25(&self) -> bool {
        self.bm25.unwrap_or(true)
    }

    pub fn get_schema(&self) -> Option<String> {
        self.schema.clone()
    }

    /// Format the config with the provided introspection data and secondary indices.
    /// This method is used during code generation to embed schema metadata.
    pub fn fmt_with_schema(
        &self,
        f: &mut fmt::Formatter,
        introspection_data: Option<&IntrospectionData>,
        secondary_indices: &[SecondaryIndex],
        hql_schema_raw: Option<&str>,
    ) -> fmt::Result {
        writeln!(f, "pub fn config() -> Option<Config> {{")?;
        writeln!(f, "return Some(Config {{")?;
        writeln!(f, "vector_config: Some(VectorConfig {{")?;
        writeln!(
            f,
            "m: Some({}),",
            self.vector_config
                .as_ref()
                .unwrap_or(&VectorConfig::default())
                .m
                .unwrap_or(16)
        )?;
        writeln!(
            f,
            "ef_construction: Some({}),",
            self.vector_config
                .as_ref()
                .unwrap_or(&VectorConfig::default())
                .ef_construction
                .unwrap_or(128)
        )?;
        writeln!(
            f,
            "ef_search: Some({}),",
            self.vector_config
                .as_ref()
                .unwrap_or(&VectorConfig::default())
                .ef_search
                .unwrap_or(768)
        )?;
        writeln!(f, "}}),")?;
        writeln!(f, "graph_config: Some(GraphConfig {{")?;
        writeln!(
            f,
            "secondary_indices: {},",
            if secondary_indices.is_empty() {
                "None".to_string()
            } else {
                format!(
                    "Some(vec![{}])",
                    secondary_indices
                        .iter()
                        .map(|i| format!("{i}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        )?;
        writeln!(f, "}}),")?;
        writeln!(
            f,
            "db_max_size_gb: Some({}),",
            self.db_max_size_gb.unwrap_or(4)
        )?;
        writeln!(f, "mcp: Some({}),", self.mcp.unwrap_or(true))?;
        writeln!(f, "bm25: Some({}),", self.bm25.unwrap_or(true))?;
        if let Some(data) = introspection_data
            && let Ok(stringified) = sonic_rs::to_string_pretty(data)
        {
            writeln!(f, "schema: Some(r#\"{stringified}\"#.to_string()),")?;
        } else {
            writeln!(f, "schema: None,")?;
        }
        writeln!(
            f,
            "embedding_model: {},",
            match &self.embedding_model {
                Some(model) => format!("Some(\"{model}\".to_string())"),
                None => "None".to_string(),
            }
        )?;
        writeln!(
            f,
            "graphvis_node_label: {},",
            match &self.graphvis_node_label {
                Some(label) => format!("Some(\"{label}\".to_string())"),
                None => "None".to_string(),
            }
        )?;
        match hql_schema_raw {
            Some(raw) => {
                let n = required_raw_str_hashes(raw);
                let hashes = "#".repeat(n);
                writeln!(f, "hql_schema_raw: Some(r{hashes}\"{raw}\"{hashes}.to_string()),")?;
            }
            None => writeln!(f, "hql_schema_raw: None,")?,
        }
        writeln!(f, "}})")?;
        writeln!(f, "}}")?;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            vector_config: Some(VectorConfig {
                m: Some(16),
                ef_construction: Some(128),
                ef_search: Some(768),
            }),
            graph_config: Some(GraphConfig {
                secondary_indices: None,
            }),
            db_max_size_gb: Some(4),
            mcp: Some(true),
            bm25: Some(true),
            schema: None,
            embedding_model: Some("text-embedding-ada-002".to_string()),
            graphvis_node_label: None,
            hql_schema_raw: None,
        }
    }
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // For backward compatibility, delegate to fmt_with_schema with empty values.
        // The actual introspection data and secondary indices should be provided
        // via fmt_with_schema when generating code from Source.
        self.fmt_with_schema(f, None, &[], None)
    }
}

#[cfg(test)]
mod runtime_schema_tests {
    use super::*;

    #[test]
    fn test_config_has_hql_schema_raw_field() {
        let cfg = Config {
            hql_schema_raw: Some("N::Foo { x: String }".to_string()),
            ..Config::default()
        };
        assert_eq!(cfg.hql_schema_raw.as_deref(), Some("N::Foo { x: String }"));
    }

    #[test]
    fn test_config_default_has_no_hql_schema_raw() {
        let cfg = Config::default();
        assert!(cfg.hql_schema_raw.is_none());
    }

    #[test]
    fn test_fmt_with_schema_embeds_hql_schema_raw() {
        let cfg = Config::default();
        let rendered = format!("{cfg}");
        assert!(rendered.contains("hql_schema_raw: None"));
    }

    #[test]
    fn test_fmt_with_schema_embeds_hql_schema_raw_some() {
        // A raw string that contains "# to exercise the dynamic hash guard
        let tricky = "N::Foo { x: String } // test \"# edge case";
        let cfg = Config::default();

        struct Wrapper<'a>(&'a Config, &'a str);
        impl<'a> std::fmt::Display for Wrapper<'a> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt_with_schema(f, None, &[], Some(self.1))
            }
        }
        let rendered = format!("{}", Wrapper(&cfg, tricky));
        // The rendered output should contain the raw string with at least 2 hashes
        assert!(
            rendered.contains("hql_schema_raw: Some(r##\""),
            "expected r## guard for content with \"#, got: {rendered}"
        );
        assert!(
            rendered.contains("##.to_string())"),
            "closing hashes not found in: {rendered}"
        );
    }
}
