use crate::{
    protocol::Response,
    sparrow_engine::types::GraphError,
    sparrow_gateway::router::router::HandlerInput,
};
use super::RuntimeError;

pub fn handle(input: HandlerInput, hql_schema_raw: Option<String>) -> Result<Response, GraphError> {
    let _ = input;
    let _ = hql_schema_raw;
    // Placeholder: parse/lower/execute will be wired in Tasks 3 and 4
    Err(GraphError::DecodeError(
        "runtime eval: not yet fully implemented".to_string()
    ))
}
