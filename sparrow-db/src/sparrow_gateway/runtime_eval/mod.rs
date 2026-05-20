use serde::{Deserialize, Serialize};

pub mod handler;
pub mod parse;
pub mod lower;
pub mod executor;

#[derive(Debug, Deserialize)]
pub struct RuntimeEvalRequest {
    pub query: String,
    pub params: sonic_rs::Value,
}

#[derive(Debug, Serialize)]
pub struct RuntimeEvalResponse {
    pub result: sonic_rs::Value,
}

#[derive(Debug)]
pub enum RuntimeError {
    Parse(String),
    Analysis(String),
    Lowering(String),
    Execution(crate::sparrow_engine::types::GraphError),
    NoSchema,
    NoQuery,
    Unsupported(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Parse(s) => write!(f, "parse error: {s}"),
            RuntimeError::Analysis(s) => write!(f, "analysis error: {s}"),
            RuntimeError::Lowering(s) => write!(f, "lowering error: {s}"),
            RuntimeError::Execution(e) => write!(f, "execution error: {e}"),
            RuntimeError::NoSchema => write!(f, "no schema available — container was not compiled with SPARROW_RUNTIME_HQL support"),
            RuntimeError::NoQuery => write!(f, "request contains no query"),
            RuntimeError::Unsupported(s) => write!(f, "unsupported expression: {s}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<RuntimeError> for crate::sparrow_engine::types::GraphError {
    fn from(e: RuntimeError) -> Self {
        crate::sparrow_engine::types::GraphError::DecodeError(e.to_string())
    }
}

impl From<crate::sparrow_engine::types::GraphError> for RuntimeError {
    fn from(e: crate::sparrow_engine::types::GraphError) -> Self {
        RuntimeError::Execution(e)
    }
}
