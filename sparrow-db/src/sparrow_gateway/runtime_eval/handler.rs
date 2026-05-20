use crate::{
    protocol::{value::Value, Format, Response},
    sparrow_engine::types::GraphError,
    sparrow_gateway::router::router::HandlerInput,
};
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use std::collections::HashMap;
use super::{
    executor::execute_plan,
    lower::lower_query,
    parse::parse_and_validate,
    RuntimeEvalRequest, RuntimeError,
};

pub fn handle(input: HandlerInput, hql_schema_raw: Option<String>) -> Result<Response, GraphError> {
    let schema = hql_schema_raw.ok_or(RuntimeError::NoSchema)?;

    let req: RuntimeEvalRequest = Format::Json
        .deserialize_owned(&input.request.body)
        .map_err(|e| GraphError::DecodeError(e.to_string()))?;

    if req.query.trim().is_empty() {
        return Err(RuntimeError::NoQuery.into());
    }

    // hql_schema_raw embeds all compiled stub queries, so source.queries after
    // parse_and_validate always has stubs + submitted query — is_empty() won't
    // catch a missing QUERY keyword. Check the submitted text directly.
    if !req.query.contains("QUERY ") {
        return Err(GraphError::DecodeError(
            "request must contain a QUERY definition".into(),
        ));
    }

    let params = parse_params(&req.params)?;
    let source =
        parse_and_validate(&schema, &req.query).map_err(GraphError::from)?;

    // Coerce JSON integers to match declared query parameter types
    let mut params = params;
    coerce_params_to_schema(&mut params, &source);

    let (steps, return_vars) = lower_query(&source, &params).map_err(GraphError::from)?;
    let result =
        execute_plan(&steps, &return_vars, &input.graph.storage).map_err(GraphError::from)?;

    let body = sonic_rs::to_vec(&result)
        .map_err(|e| GraphError::DecodeError(e.to_string()))?;

    Ok(Response {
        body,
        fmt: Format::Json,
    })
}

fn coerce_params_to_schema(
    params: &mut HashMap<String, Value>,
    source: &crate::sparrowc::parser::types::Source,
) {
    // Submitted query is always appended last in the combined source.
    let query = match source.queries.last() {
        Some(q) => q,
        None => return,
    };
    for param in &query.parameters {
        let name = &param.name.1;
        let field_type = &param.param_type.1;
        if let Some(val) = params.get_mut(name) {
            coerce_value(val, field_type);
        }
    }
}

fn coerce_value(val: &mut Value, field_type: &crate::sparrowc::parser::types::FieldType) {
    use crate::sparrowc::parser::types::FieldType;
    match (val.clone(), field_type) {
        (Value::I64(n), FieldType::I32) => {
            *val = Value::I32(n as i32);
        }
        (Value::I64(n), FieldType::I8) => {
            *val = Value::I32(n as i32); // I8 maps to I32 in storage
        }
        (Value::I64(n), FieldType::I16) => {
            *val = Value::I32(n as i32);
        }
        (Value::I64(n), FieldType::U32) => {
            *val = Value::U64(n as u64);
        }
        (Value::I64(n), FieldType::U64) => {
            *val = Value::U64(n as u64);
        }
        _ => {} // I64, F64, String, Boolean are already correct
    }
}

fn parse_params(
    params: &sonic_rs::Value,
) -> Result<HashMap<String, Value>, GraphError> {
    let mut map = HashMap::new();
    if let Some(obj) = params.as_object() {
        for (k, v) in obj.iter() {
            let val = json_val_to_value(v)
                .map_err(|e| GraphError::DecodeError(format!("param '{k}': {e}")))?;
            map.insert(k.to_string(), val);
        }
    }
    Ok(map)
}

fn json_val_to_value(v: &sonic_rs::Value) -> Result<Value, String> {
    if v.is_str() {
        Ok(Value::String(v.as_str().unwrap().to_string()))
    } else if let Some(i) = v.as_i64() {
        Ok(Value::I64(i))
    } else if let Some(u) = v.as_u64() {
        Ok(Value::U64(u))
    } else if let Some(f) = v.as_f64() {
        Ok(Value::F64(f))
    } else if let Some(b) = v.as_bool() {
        Ok(Value::Boolean(b))
    } else {
        Err(format!("unsupported JSON param type: {v:?}"))
    }
}
