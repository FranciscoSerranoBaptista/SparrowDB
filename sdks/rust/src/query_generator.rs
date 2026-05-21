use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Current wire-format version for generated query bundles.
pub const QUERY_BUNDLE_VERSION: u32 = 4;

/// Declared shape of a registered query parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryParamType {
    /// Boolean parameter.
    Bool,
    /// 64-bit signed integer parameter.
    I64,
    /// 64-bit floating point parameter.
    F64,
    /// 32-bit floating point parameter.
    F32,
    /// UTF-8 string parameter.
    String,
    /// RFC3339 datetime parameter normalized to UTC.
    DateTime,
    /// Raw bytes parameter.
    Bytes,
    /// Any nested `PropertyValue` payload.
    Value,
    /// Object/map payload.
    Object,
    /// Array payload whose elements have the given shape.
    Array(Box<QueryParamType>),
}

/// Declared parameter for a registered query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryParameter {
    /// Parameter name.
    pub name: String,
    /// Parameter shape.
    pub ty: QueryParamType,
}

/// Versioned payload written to `queries.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryBundle {
    /// Wire-format version.
    pub version: u32,
    /// Read-only query routes by route name.
    pub read_routes: BTreeMap<String, crate::ReadBatch>,
    /// Write-capable query routes by route name.
    pub write_routes: BTreeMap<String, crate::WriteBatch>,
    /// Registered read-route parameter metadata.
    pub read_parameters: BTreeMap<String, Vec<QueryParameter>>,
    /// Registered write-route parameter metadata.
    pub write_parameters: BTreeMap<String, Vec<QueryParameter>>,
}

impl Default for QueryBundle {
    fn default() -> Self {
        Self {
            version: QUERY_BUNDLE_VERSION,
            read_routes: BTreeMap::new(),
            write_routes: BTreeMap::new(),
            read_parameters: BTreeMap::new(),
            write_parameters: BTreeMap::new(),
        }
    }
}

/// Registered read-query function.
pub struct RegisteredReadQuery {
    /// Route name.
    pub name: &'static str,
    /// Function that constructs the route AST.
    pub build: fn() -> crate::ReadBatch,
    /// Function that constructs declared parameter metadata.
    pub parameters: fn() -> Vec<QueryParameter>,
}

/// Registered write-query function.
pub struct RegisteredWriteQuery {
    /// Route name.
    pub name: &'static str,
    /// Function that constructs the route AST.
    pub build: fn() -> crate::WriteBatch,
    /// Function that constructs declared parameter metadata.
    pub parameters: fn() -> Vec<QueryParameter>,
}

inventory::collect!(RegisteredReadQuery);
inventory::collect!(RegisteredWriteQuery);

/// Errors returned while generating or loading query bundles.
#[derive(Debug)]
pub enum GenerateError {
    /// More than one query registered the same route name.
    DuplicateQueryName(String),
    /// Failed to read or write bundle file.
    Io(std::io::Error),
    /// Failed to serialize or deserialize the bundle.
    Json(sonic_rs::Error),
    /// Bundle version is unsupported.
    UnsupportedVersion {
        /// Version read from payload.
        found: u32,
        /// Version required by this crate.
        expected: u32,
    },
}

impl std::fmt::Display for GenerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateQueryName(name) => {
                write!(f, "duplicate generated query name: {name}")
            }
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Json(err) => write!(f, "json error: {err}"),
            Self::UnsupportedVersion { found, expected } => {
                write!(
                    f,
                    "unsupported query bundle version {found} (expected {expected})"
                )
            }
        }
    }
}

impl std::error::Error for GenerateError {}

impl From<std::io::Error> for GenerateError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<sonic_rs::Error> for GenerateError {
    fn from(value: sonic_rs::Error) -> Self {
        Self::Json(value)
    }
}

/// Build the in-memory query bundle from all `#[register]` registrations.
pub fn build_query_bundle() -> Result<QueryBundle, GenerateError> {
    let mut bundle = QueryBundle::default();

    for registered in inventory::iter::<RegisteredReadQuery> {
        if bundle.read_routes.contains_key(registered.name)
            || bundle.write_routes.contains_key(registered.name)
        {
            return Err(GenerateError::DuplicateQueryName(
                registered.name.to_string(),
            ));
        }

        bundle
            .read_routes
            .insert(registered.name.to_string(), (registered.build)());
        bundle
            .read_parameters
            .insert(registered.name.to_string(), (registered.parameters)());
    }

    for registered in inventory::iter::<RegisteredWriteQuery> {
        if bundle.read_routes.contains_key(registered.name)
            || bundle.write_routes.contains_key(registered.name)
        {
            return Err(GenerateError::DuplicateQueryName(
                registered.name.to_string(),
            ));
        }

        bundle
            .write_routes
            .insert(registered.name.to_string(), (registered.build)());
        bundle
            .write_parameters
            .insert(registered.name.to_string(), (registered.parameters)());
    }

    Ok(bundle)
}

/// Serialize a query bundle to JSON bytes.
pub fn serialize_query_bundle(bundle: &QueryBundle) -> Result<Vec<u8>, GenerateError> {
    Ok(sonic_rs::to_vec_pretty(bundle)?)
}

/// Deserialize a query bundle from JSON bytes.
pub fn deserialize_query_bundle(bytes: &[u8]) -> Result<QueryBundle, GenerateError> {
    let bundle: QueryBundle = sonic_rs::from_slice(bytes)?;

    if bundle.version != QUERY_BUNDLE_VERSION {
        return Err(GenerateError::UnsupportedVersion {
            found: bundle.version,
            expected: QUERY_BUNDLE_VERSION,
        });
    }

    Ok(bundle)
}

/// Write a query bundle to a file.
pub fn write_query_bundle_to_path<P: AsRef<Path>>(
    bundle: &QueryBundle,
    path: P,
) -> Result<(), GenerateError> {
    let bytes = serialize_query_bundle(bundle)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Read a query bundle from a file.
pub fn read_query_bundle_from_path<P: AsRef<Path>>(path: P) -> Result<QueryBundle, GenerateError> {
    let bytes = std::fs::read(path)?;
    deserialize_query_bundle(&bytes)
}

/// Generate `queries.json` in the current working directory.
pub fn generate() -> Result<PathBuf, GenerateError> {
    generate_to_path("queries.json")
}

/// Generate a query bundle and write it to the requested output path.
pub fn generate_to_path<P: AsRef<Path>>(path: P) -> Result<PathBuf, GenerateError> {
    let path = path.as_ref();
    let bundle = build_query_bundle()?;
    write_query_bundle_to_path(&bundle, path)?;
    Ok(path.to_path_buf())
}
