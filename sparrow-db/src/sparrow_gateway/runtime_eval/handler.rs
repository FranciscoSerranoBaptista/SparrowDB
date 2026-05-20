use crate::{
    protocol::{Format, Response},
    sparrow_engine::types::GraphError,
    sparrow_gateway::router::router::HandlerInput,
};
use super::{RuntimeEvalRequest, RuntimeError, parse::parse_and_validate};

pub fn handle(input: HandlerInput, hql_schema_raw: Option<String>) -> Result<Response, GraphError> {
    let schema = hql_schema_raw.ok_or(RuntimeError::NoSchema)?;

    let req: RuntimeEvalRequest = Format::Json
        .deserialize_owned(&input.request.body)
        .map_err(|e| GraphError::DecodeError(e.to_string()))?;

    if req.query.trim().is_empty() {
        return Err(RuntimeError::NoQuery.into());
    }

    let source = parse_and_validate(&schema, &req.query)
        .map_err(GraphError::from)?;

    if source.queries.is_empty() {
        return Err(GraphError::DecodeError(
            "request must contain exactly one QUERY".into(),
        ));
    }

    // Placeholder response until Task 4 wires in execution
    #[derive(serde::Serialize)]
    struct ParsedOk<'a> {
        status: &'a str,
        query: &'a str,
    }
    let response_body = sonic_rs::to_vec(&ParsedOk {
        status: "parsed_ok",
        query: &source.queries[0].name,
    })
    .map_err(|e| GraphError::DecodeError(e.to_string()))?;

    Ok(Response {
        body: response_body,
        fmt: Format::Json,
    })
}
