/// HelixDB v1/query compatibility endpoint.
///
/// Translates the HelixDB JSON DSL into SparrowDB storage operations so simorgh
/// can migrate data without rewriting its query layer first.
///
/// See docs/V1_COMPAT_ENDPOINT.md for the full design rationale and migration guide.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::IntoResponse;
use bumpalo::Bump;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use tracing::{info, warn};

use crate::{
    protocol::{value::Value, Format, Response, request::RequestType},
    sparrow_engine::{
        storage_core::{storage_methods::StorageMethods, SparrowGraphStorage},
        traversal_core::traversal_value::TraversalValue,
        types::GraphError,
    },
    sparrow_gateway::{
        gateway::AppState,
        mcp::tools::{
            EdgeType, FilterProperties, FilterTraversal, Operator, ToolArgs,
            execute_query_chain, execute_query_chain_from_seed,
        },
        router::router::{Handler, HandlerInput, HandlerSubmission},
    },
    utils::properties::ImmutablePropertiesMap,
};

use crate::sparrow_engine::traversal_core::ops::{
    g::G,
    source::{add_e::AddEAdapter, add_n::AddNAdapter},
};

// ─── handler registrations ────────────────────────────────────────────────────

inventory::submit! {
    HandlerSubmission(Handler::new("__v1_compat_read", v1_compat_handler, false))
}
inventory::submit! {
    HandlerSubmission(Handler::new("__v1_compat_write", v1_compat_handler, true))
}

/// Axum handler for `POST /v1/query`.
///
/// Peeks at `request_type` in the body to route to the read or write worker,
/// then delegates to the registered `__v1_compat_read` / `__v1_compat_write`
/// handlers via the worker pool (preserving LMDB single-writer safety).
pub async fn v1_query_axum_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> axum::http::Response<Body> {
    let start = Instant::now();

    // Cheap check: does the body contain `"write"` as the request_type?
    let is_write = body.windows(b"\"write\"".len()).any(|w| w == b"\"write\"");
    let handler_name = if is_write { "__v1_compat_write" } else { "__v1_compat_read" };

    #[cfg(feature = "lmdb")]
    {
        if state.token_store.is_auth_required() {
            let raw_key = headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            match state.token_store.verify(raw_key) {
                Ok(record) => {
                    if is_write && !record.role.can_write() {
                        use crate::protocol::SparrowError;
                        return SparrowError::Forbidden.into_response();
                    }
                }
                Err(_) => {
                    use crate::protocol::SparrowError;
                    return SparrowError::InvalidApiKey.into_response();
                }
            }
        }
    }

    // Suppress unused variable warning when lmdb feature is disabled.
    let _ = &headers;

    let req = crate::protocol::request::Request {
        name: handler_name.to_string(),
        req_type: RequestType::Query,
        api_key: None,
        body,
        in_fmt: Format::Json,
        out_fmt: Format::Json,
        pre_computed_embedding: None,
    };

    match state.worker_pool.process(req).await {
        Ok(r) => {
            info!(handler = handler_name, elapsed_us = start.elapsed().as_micros(), "v1_compat ok");
            r.into_response()
        }
        Err(e) => {
            info!(handler = handler_name, error = ?e, "v1_compat error");
            e.into_response()
        }
    }
}

pub fn v1_compat_handler(input: HandlerInput) -> Result<Response, GraphError> {
    let body: sonic_rs::Value = sonic_rs::from_slice(&input.request.body)
        .map_err(|e| GraphError::DecodeError(format!("v1_compat: invalid JSON: {e}")))?;

    let query = body.get("query").ok_or_else(|| {
        GraphError::DecodeError("v1_compat: missing 'query' field".to_string())
    })?;

    let raw_queries = query
        .get("queries")
        .and_then(|q| q.as_array())
        .ok_or_else(|| {
            GraphError::DecodeError("v1_compat: missing 'queries' array".to_string())
        })?;

    let return_vars: Vec<String> = query
        .get("returns")
        .and_then(|r| r.as_array())
        .map(|a| &**a)
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();

    let arena = Bump::new();
    let result = execute_helix_queries(raw_queries, &input.graph.storage, &arena)
        .map_err(|e| GraphError::DecodeError(e.to_string()))?;

    let mut output: HashMap<String, sonic_rs::Value> = HashMap::new();
    for var_name in &return_vars {
        if let Some(nodes) = result.get(var_name) {
            let ids: Vec<sonic_rs::Value> = nodes
                .iter()
                .filter_map(|n| n.get("id"))
                .map(|v| v.clone())
                .collect();
            let entry = sonic_rs::json!({
                "ids": ids,
                "properties": nodes
            });
            output.insert(var_name.clone(), entry);
        }
    }

    let body_bytes = sonic_rs::to_vec(&output)
        .map_err(|e| GraphError::DecodeError(format!("v1_compat: serialise: {e}")))?;

    Ok(Response { body: body_bytes, fmt: Format::Json })
}

// ─── compatibility error ───────────────────────────────────────────────────────

#[derive(Debug)]
enum CompatError {
    Translation(String),
    Execution(String),
}

impl std::fmt::Display for CompatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompatError::Translation(s) => write!(f, "v1_compat translation: {s}"),
            CompatError::Execution(s) => write!(f, "v1_compat execution: {s}"),
        }
    }
}

impl From<GraphError> for CompatError {
    fn from(e: GraphError) -> Self {
        CompatError::Execution(e.to_string())
    }
}

// ─── internal op model ────────────────────────────────────────────────────────

/// A resolved traversal or mutation step for one named query.
enum CompatStep {
    /// Traverse the graph, optionally seeded from a prior result.
    Traverse {
        seed_var: Option<String>,
        tool_args: Vec<ToolArgs>,
        bind_to: String,
    },
    /// Look up nodes directly by UUID string.
    LookupByUuid {
        ids: Vec<u128>,
        bind_to: String,
    },
    AddNode {
        node_type: String,
        fields: HashMap<String, Value>,
        bind_to: String,
    },
    AddEdge {
        edge_type: String,
        from_var: String,
        to_var: String,
        fields: HashMap<String, Value>,
        bind_to: String,
    },
    UpdateProperties {
        seed_var: Option<String>,
        tool_args: Vec<ToolArgs>,
        updates: HashMap<String, Value>,
        bind_to: String,
    },
    DropNodes {
        seed_var: Option<String>,
        tool_args: Vec<ToolArgs>,
    },
}

// ─── multi-query translation ───────────────────────────────────────────────────

/// Translate and execute all named queries from a HelixDB request.
///
/// Results are keyed by their query name and each value is a list of serialised
/// node JSON objects with `$id` and `$label` compat aliases included.
fn execute_helix_queries<'db, 'arena>(
    raw_queries: &[sonic_rs::Value],
    storage: &'db SparrowGraphStorage,
    arena: &'arena Bump,
) -> Result<HashMap<String, Vec<sonic_rs::Value>>, CompatError>
where
    'db: 'arena,
{
    let mut live_store: HashMap<String, Vec<TraversalValue<'arena>>> = HashMap::new();
    let mut result_store: HashMap<String, Vec<sonic_rs::Value>> = HashMap::new();

    for query_item in raw_queries {
        let query_body = match query_item.get("Query") {
            Some(q) => q,
            None => continue,
        };

        let name = query_body
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("result");

        let raw_steps = query_body
            .get("steps")
            .and_then(|s| s.as_array())
            .map(|a| &**a)
            .unwrap_or(&[]);

        let steps = translate_named_query(name, raw_steps)?;

        for step in steps {
            execute_compat_step(step, storage, arena, &mut live_store, &mut result_store)?;
        }
    }

    Ok(result_store)
}

fn translate_named_query(
    name: &str,
    raw_steps: &[sonic_rs::Value],
) -> Result<Vec<CompatStep>, CompatError> {
    let mut out: Vec<CompatStep> = Vec::new();

    // State accumulated across steps within this named query.
    let mut seed_var: Option<String> = None;
    let mut tool_args: Vec<ToolArgs> = Vec::new();
    // When a step is N:{Ids:[...]}, we need a UUID lookup, not a traversal.
    let mut pending_uuid_ids: Option<Vec<u128>> = None;
    // Accumulate SetProperty updates until we see the end of the query.
    let mut pending_updates: Vec<(String, Value)> = Vec::new();

    for step in raw_steps {
        // ── N:{Ids:[...]} ──────────────────────────────────────────────────
        if let Some(ids_val) = step.get("N").and_then(|n| n.get("Ids")) {
            flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
            let ids = parse_uuid_ids(ids_val)?;
            pending_uuid_ids = Some(ids);
            continue;
        }

        // ── NWhere:{...} ───────────────────────────────────────────────────
        if let Some(cond) = step.get("NWhere") {
            flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
            if let Some(uuids) = pending_uuid_ids.take() {
                out.push(CompatStep::LookupByUuid { ids: uuids, bind_to: name.to_string() });
                seed_var = Some(name.to_string());
            }
            let args = translate_nwhere(cond)?;
            tool_args.extend(args);
            continue;
        }

        // ── Inject:"varname" ───────────────────────────────────────────────
        if let Some(inject_name) = step.get("Inject").and_then(|v| v.as_str()) {
            flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
            if let Some(uuids) = pending_uuid_ids.take() {
                out.push(CompatStep::LookupByUuid { ids: uuids, bind_to: name.to_string() });
            }
            seed_var = Some(inject_name.to_string());
            continue;
        }

        // ── Out:"EDGE" ─────────────────────────────────────────────────────
        if let Some(edge) = step.get("Out").and_then(|v| v.as_str()) {
            maybe_flush_uuid_lookup(&mut out, &mut pending_uuid_ids, &mut seed_var, name)?;
            tool_args.push(ToolArgs::OutStep {
                edge_label: edge.to_string(),
                edge_type: EdgeType::Node,
                filter: None,
            });
            continue;
        }

        // ── In:"EDGE" ──────────────────────────────────────────────────────
        if let Some(edge) = step.get("In").and_then(|v| v.as_str()) {
            maybe_flush_uuid_lookup(&mut out, &mut pending_uuid_ids, &mut seed_var, name)?;
            tool_args.push(ToolArgs::InStep {
                edge_label: edge.to_string(),
                edge_type: EdgeType::Node,
                filter: None,
            });
            continue;
        }

        // ── Where:{...} ────────────────────────────────────────────────────
        if let Some(cond) = step.get("Where") {
            maybe_flush_uuid_lookup(&mut out, &mut pending_uuid_ids, &mut seed_var, name)?;
            let filter_arg = translate_where_condition(cond)?;
            tool_args.push(filter_arg);
            continue;
        }

        // ── AddN:{label, properties} ───────────────────────────────────────
        if let Some(add_n) = step.get("AddN") {
            flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
            if let Some(uuids) = pending_uuid_ids.take() {
                out.push(CompatStep::LookupByUuid { ids: uuids, bind_to: name.to_string() });
            }
            let node_type = add_n
                .get("label")
                .and_then(|l| l.as_str())
                .ok_or_else(|| CompatError::Translation("AddN missing label".to_string()))?
                .to_string();
            let fields = if let Some(props) = add_n.get("properties") {
                parse_node_properties(props)?
            } else {
                HashMap::new()
            };
            out.push(CompatStep::AddNode { node_type, fields, bind_to: name.to_string() });
            return Ok(out);
        }

        // ── AddE:{label, to:{Var:T}, properties} ──────────────────────────
        if let Some(add_e) = step.get("AddE") {
            let from_var = seed_var.take().ok_or_else(|| {
                CompatError::Translation("AddE requires Inject before it".to_string())
            })?;
            let to_var = add_e
                .get("to")
                .and_then(|t| t.get("Var"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    CompatError::Translation("AddE: to.Var is required".to_string())
                })?
                .to_string();
            let edge_type = add_e
                .get("label")
                .and_then(|l| l.as_str())
                .unwrap_or("")
                .to_string();
            let fields = if let Some(props) = add_e.get("properties") {
                parse_edge_properties(props)?
            } else {
                HashMap::new()
            };
            out.push(CompatStep::AddEdge { edge_type, from_var, to_var, fields, bind_to: name.to_string() });
            return Ok(out);
        }

        // ── SetProperty:["key", val] ───────────────────────────────────────
        if let Some(set_prop) = step.get("SetProperty") {
            let arr = set_prop.as_array().map(|a| &**a).unwrap_or(&[]);
            if arr.len() == 2 {
                let key = arr[0].as_str().unwrap_or("").to_string();
                let val = parse_property_input(&arr[1])?;
                pending_updates.push((key, val));
            }
            continue;
        }

        // ── Drop:null ──────────────────────────────────────────────────────
        if step.get("Drop").is_some() {
            maybe_flush_uuid_lookup(&mut out, &mut pending_uuid_ids, &mut seed_var, name)?;
            let (sv, args) = (seed_var.take(), std::mem::take(&mut tool_args));
            out.push(CompatStep::DropNodes { seed_var: sv, tool_args: args });
            return Ok(out);
        }

        // ── VectorSearchNodes:{label, property, query_vector, k} ──────────
        if let Some(vs) = step.get("VectorSearchNodes") {
            flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
            if let Some(uuids) = pending_uuid_ids.take() {
                out.push(CompatStep::LookupByUuid { ids: uuids, bind_to: name.to_string() });
                seed_var = Some(name.to_string());
            }
            let args = translate_vector_search(vs)?;
            tool_args.extend(args);
            continue;
        }

        // ── Id:null, ValueMap:[...], Project:[...], Count ─────────────────
        // These are result-shaping operations. We always return all fields and
        // let the caller select what it needs. No-op in v1_compat.
        if step.get("Id").is_some()
            || step.get("ValueMap").is_some()
            || step.get("Project").is_some()
            || step.as_str() == Some("Count")
        {
            continue;
        }

        // ── Unknown step ───────────────────────────────────────────────────
        warn!(step = %step, "v1_compat: unrecognised step — skipping");
    }

    // Flush any accumulated state at end of query.
    if let Some(uuids) = pending_uuid_ids.take() {
        flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
        out.push(CompatStep::LookupByUuid { ids: uuids, bind_to: name.to_string() });
    } else if !pending_updates.is_empty() {
        let updates: HashMap<String, Value> = pending_updates.into_iter().collect();
        let (sv, args) = (seed_var.take(), std::mem::take(&mut tool_args));
        out.push(CompatStep::UpdateProperties {
            seed_var: sv,
            tool_args: args,
            updates,
            bind_to: name.to_string(),
        });
    } else {
        flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
    }

    Ok(out)
}

/// Flush accumulated traversal state as a Traverse step if there's anything to flush.
fn flush_traversal(
    out: &mut Vec<CompatStep>,
    seed_var: &mut Option<String>,
    tool_args: &mut Vec<ToolArgs>,
    bind_to: &str,
) -> Result<(), CompatError> {
    if seed_var.is_some() || !tool_args.is_empty() {
        out.push(CompatStep::Traverse {
            seed_var: seed_var.take(),
            tool_args: std::mem::take(tool_args),
            bind_to: bind_to.to_string(),
        });
    }
    Ok(())
}

/// If there are pending UUID lookups, flush them as a LookupByUuid step and update seed_var.
fn maybe_flush_uuid_lookup(
    out: &mut Vec<CompatStep>,
    pending: &mut Option<Vec<u128>>,
    seed_var: &mut Option<String>,
    bind_to: &str,
) -> Result<(), CompatError> {
    if let Some(uuids) = pending.take() {
        out.push(CompatStep::LookupByUuid {
            ids: uuids,
            bind_to: bind_to.to_string(),
        });
        *seed_var = Some(bind_to.to_string());
    }
    Ok(())
}

// ─── step translation ──────────────────────────────────────────────────────────

/// Translate a `NWhere` condition object into tool args.
///
/// The HelixDB DSL embeds the label type inside the condition:
/// `{"And": [{"Eq": ["$label", {"String": T}]}, ...]}` → `[NFromType(T), FilterItems(...)]`
fn translate_nwhere(cond: &sonic_rs::Value) -> Result<Vec<ToolArgs>, CompatError> {
    if let Some(and_arr) = cond.get("And").and_then(|a| a.as_array()) {
        // Extract label eq if present.
        let label_type = and_arr.iter().find_map(label_from_eq);
        let rest: Vec<&sonic_rs::Value> = and_arr
            .iter()
            .filter(|c| label_from_eq(c).is_none())
            .collect();

        let mut args = Vec::new();
        if let Some(label) = label_type {
            args.push(ToolArgs::NFromType { node_type: label });
        }
        let filter_props = rest
            .into_iter()
            .map(translate_eq_condition)
            .collect::<Result<Vec<_>, _>>()?;
        let filter_props: Vec<FilterProperties> = filter_props.into_iter().flatten().flatten().collect();
        if !filter_props.is_empty() {
            args.push(ToolArgs::FilterItems {
                filter: FilterTraversal {
                    properties: Some(vec![filter_props]),
                    filter_traversals: None,
                },
            });
        }
        return Ok(args);
    }

    if cond.get("Eq").is_some() {
        if let Some(label) = label_from_eq(cond) {
            return Ok(vec![ToolArgs::NFromType { node_type: label }]);
        }
        let props = translate_eq_condition(cond)?.unwrap_or_default();
        if props.is_empty() {
            return Ok(vec![]);
        }
        return Ok(vec![ToolArgs::FilterItems {
            filter: FilterTraversal {
                properties: Some(vec![props]),
                filter_traversals: None,
            },
        }]);
    }

    Err(CompatError::Translation(format!(
        "unsupported NWhere condition: {cond}"
    )))
}

/// If this value is `{"Eq": ["$label", {"String": T}]}`, return Some(T).
fn label_from_eq(v: &sonic_rs::Value) -> Option<String> {
    let eq = v.get("Eq")?.as_array()?;
    if eq.len() == 2 && eq[0].as_str() == Some("$label") {
        return eq[1].get("String").and_then(|s| s.as_str()).map(str::to_owned);
    }
    None
}

/// Translate a single `{"Eq":["key", val]}` (or Gt/Lt/etc.) into a list of `FilterProperties`.
/// Returns `None` entries are silently dropped (e.g. unsupported operators).
fn translate_eq_condition(
    cond: &sonic_rs::Value,
) -> Result<Option<Vec<FilterProperties>>, CompatError> {
    for (op_key, sparrow_op) in &[
        ("Eq", Operator::Eq),
        ("Neq", Operator::Neq),
        ("Gt", Operator::Gt),
        ("Lt", Operator::Lt),
        ("Gte", Operator::Gte),
        ("Lte", Operator::Lte),
    ] {
        if let Some(arr) = cond.get(op_key).and_then(|a| a.as_array()) {
            if arr.len() == 2 {
                let raw_key = arr[0].as_str().unwrap_or("");
                // Map HelixDB meta-properties to SparrowDB field names.
                let key = match raw_key {
                    "$label" => "label",
                    "$id" => "id",
                    other => other,
                };
                let val = parse_typed_value(&arr[1])?;
                return Ok(Some(vec![FilterProperties {
                    key: key.to_string(),
                    value: val,
                    operator: Some(*sparrow_op),
                }]));
            }
        }
    }
    Ok(None)
}

/// Translate a `Where` condition to a `FilterItems` ToolArgs.
fn translate_where_condition(cond: &sonic_rs::Value) -> Result<ToolArgs, CompatError> {
    let props = translate_eq_condition(cond)?.unwrap_or_default();

    Ok(ToolArgs::FilterItems {
        filter: FilterTraversal {
            properties: if props.is_empty() { None } else { Some(vec![props]) },
            filter_traversals: None,
        },
    })
}

/// Translate `VectorSearchNodes` into `[NFromType(label), SearchVec(vector, k)]`.
fn translate_vector_search(vs: &sonic_rs::Value) -> Result<Vec<ToolArgs>, CompatError> {
    let label = vs
        .get("label")
        .and_then(|l| l.as_str())
        .ok_or_else(|| CompatError::Translation("VectorSearchNodes: missing label".to_string()))?
        .to_string();

    let k = vs
        .get("k")
        .and_then(|k| k.get("Literal"))
        .and_then(|l| l.as_u64())
        .unwrap_or(10) as usize;

    let vector: Vec<f64> = vs
        .get("query_vector")
        .and_then(|qv| qv.get("Value"))
        .and_then(|v| v.get("F64Array"))
        .and_then(|a| a.as_array())
        .map(|a| &**a)
        .unwrap_or(&[])
        .iter()
        .map(|f| f.as_f64().unwrap_or(0.0))
        .collect();

    if vector.is_empty() {
        return Err(CompatError::Translation(
            "VectorSearchNodes: empty query vector".to_string(),
        ));
    }

    Ok(vec![
        ToolArgs::NFromType { node_type: label },
        ToolArgs::SearchVec { vector, k, min_score: None },
    ])
}

// ─── property value parsing ────────────────────────────────────────────────────

/// Parse a HelixDB typed value: `{"String":"s"}`, `{"I64":42}`, `{"F64":1.5}`, etc.
fn parse_typed_value(v: &sonic_rs::Value) -> Result<Value, CompatError> {
    if let Some(s) = v.get("String").and_then(|s| s.as_str()) {
        return Ok(Value::String(s.to_string()));
    }
    if let Some(i) = v.get("I64").and_then(|i| i.as_i64()) {
        return Ok(Value::I64(i));
    }
    if let Some(u) = v.get("U64").and_then(|u| u.as_u64()) {
        return Ok(Value::U64(u));
    }
    if let Some(f) = v.get("F64").and_then(|f| f.as_f64()) {
        return Ok(Value::F64(f));
    }
    if let Some(b) = v.get("Bool").and_then(|b| b.as_bool()) {
        return Ok(Value::Boolean(b));
    }
    if let Some(arr) = v.get("F64Array").and_then(|a| a.as_array()) {
        let vals: Vec<Value> = arr.iter().map(|f| Value::F64(f.as_f64().unwrap_or(0.0))).collect();
        return Ok(Value::Array(vals));
    }
    // Plain JSON scalar fallback.
    if let Some(s) = v.as_str() {
        return Ok(Value::String(s.to_string()));
    }
    if let Some(i) = v.as_i64() {
        return Ok(Value::I64(i));
    }
    if let Some(f) = v.as_f64() {
        return Ok(Value::F64(f));
    }
    if let Some(b) = v.as_bool() {
        return Ok(Value::Boolean(b));
    }
    Err(CompatError::Translation(format!("unsupported typed value: {v}")))
}

/// Parse a HelixDB property input: `{"Value": <typed>}` or a plain typed value.
fn parse_property_input(v: &sonic_rs::Value) -> Result<Value, CompatError> {
    if let Some(inner) = v.get("Value") {
        return parse_typed_value(inner);
    }
    parse_typed_value(v)
}

/// Parse the `properties` array from `AddN`: `[["key", {"Value":{...}}], ...]`.
fn parse_node_properties(
    props_arr: &sonic_rs::Value,
) -> Result<HashMap<String, Value>, CompatError> {
    let mut fields = HashMap::new();
    if let Some(arr) = props_arr.as_array() {
        for item in arr {
            let pair = item.as_array().map(|a| &**a).unwrap_or(&[]);
            if pair.len() == 2 {
                let key = pair[0].as_str().unwrap_or("").to_string();
                let val = parse_property_input(&pair[1])?;
                fields.insert(key, val);
            }
        }
    }
    Ok(fields)
}

/// Parse the `properties` array from `AddE`: same format as node properties.
fn parse_edge_properties(
    props_arr: &sonic_rs::Value,
) -> Result<HashMap<String, Value>, CompatError> {
    parse_node_properties(props_arr)
}

/// Parse `N:{Ids:[...]}` — each entry is a UUID string.
fn parse_uuid_ids(ids_val: &sonic_rs::Value) -> Result<Vec<u128>, CompatError> {
    let arr = ids_val.as_array().map(|a| &**a).unwrap_or(&[]);
    let mut ids = Vec::with_capacity(arr.len());
    for v in arr {
        if let Some(s) = v.as_str() {
            let uuid = uuid::Uuid::parse_str(s).map_err(|e| {
                CompatError::Translation(format!("invalid UUID '{s}': {e}"))
            })?;
            ids.push(uuid.as_u128());
        } else if let Some(i) = v.as_u64() {
            // Legacy i64 IDs from HelixDB — treat as raw u128. Likely wrong but don't crash.
            ids.push(i as u128);
        }
    }
    Ok(ids)
}

// ─── execution ─────────────────────────────────────────────────────────────────

fn execute_compat_step<'db, 'arena>(
    step: CompatStep,
    storage: &'db SparrowGraphStorage,
    arena: &'arena Bump,
    live_store: &mut HashMap<String, Vec<TraversalValue<'arena>>>,
    result_store: &mut HashMap<String, Vec<sonic_rs::Value>>,
) -> Result<(), CompatError>
where
    'db: 'arena,
{
    match step {
        CompatStep::Traverse { seed_var, tool_args, bind_to } => {
            let txn = storage
                .graph_env
                .read_txn()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let values: Vec<TraversalValue<'arena>> = if let Some(sv) = &seed_var {
                let seeds = live_store.get(sv.as_str()).cloned().unwrap_or_default();
                execute_query_chain_from_seed(&tool_args, storage, &txn, arena, seeds.into_iter())
                    .map_err(CompatError::from)?
                    .collect()
                    .map_err(CompatError::from)?
            } else {
                execute_query_chain(&tool_args, storage, &txn, arena)
                    .map_err(CompatError::from)?
                    .collect()
                    .map_err(CompatError::from)?
            };

            let json_values = serialise_results(&values);
            live_store.insert(bind_to.clone(), values);
            result_store.insert(bind_to, json_values);
        }

        CompatStep::LookupByUuid { ids, bind_to } => {
            let txn = storage
                .graph_env
                .read_txn()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let values: Vec<TraversalValue<'arena>> = ids
                .iter()
                .filter_map(|&id| {
                    storage.get_node(&txn, id, arena).ok().map(TraversalValue::Node)
                })
                .collect();

            let json_values = serialise_results(&values);
            live_store.insert(bind_to.clone(), values);
            result_store.insert(bind_to, json_values);
        }

        CompatStep::AddNode { node_type, fields, bind_to } => {
            let mut wtxn = storage
                .graph_env
                .write_txn()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let label: &'arena str = arena.alloc_str(&node_type);

            let sec_index_names: Vec<&'static str> = fields
                .keys()
                .filter(|k| storage.secondary_indices.contains_key(k.as_str()))
                .map(|k| Box::leak(k.clone().into_boxed_str()) as &'static str)
                .collect();
            let sec_indices: Option<&[&str]> = if sec_index_names.is_empty() {
                None
            } else {
                Some(&sec_index_names)
            };

            let count = fields.len();
            let iter = fields.iter().map(|(k, v)| (arena.alloc_str(k) as &'arena str, v.clone()));
            let props = ImmutablePropertiesMap::new(count, iter, arena);

            let result = G::new_mut(storage, arena, &mut wtxn)
                .add_n(label, Some(props), sec_indices)
                .collect_to_obj()
                .map_err(CompatError::from)?;

            wtxn.commit()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let json = sonic_rs::to_value(&result).unwrap_or_default();
            let json_with_aliases = add_dollar_aliases(json);
            live_store.insert(bind_to.clone(), vec![result]);
            result_store.insert(bind_to, vec![json_with_aliases]);
        }

        CompatStep::AddEdge { edge_type, from_var, to_var, fields, bind_to } => {
            let from_node = live_store
                .get(from_var.as_str())
                .and_then(|v| v.first())
                .cloned()
                .ok_or_else(|| {
                    CompatError::Execution(format!("AddEdge: from_var '{from_var}' not found"))
                })?;
            let to_node = live_store
                .get(to_var.as_str())
                .and_then(|v| v.first())
                .cloned()
                .ok_or_else(|| {
                    CompatError::Execution(format!("AddEdge: to_var '{to_var}' not found"))
                })?;

            let from_id = from_node.id();
            let to_id = to_node.id();

            let mut wtxn = storage
                .graph_env
                .write_txn()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let label: &'arena str = arena.alloc_str(&edge_type);
            let props = if fields.is_empty() {
                None
            } else {
                let count = fields.len();
                let iter =
                    fields.iter().map(|(k, v)| (arena.alloc_str(k) as &'arena str, v.clone()));
                Some(ImmutablePropertiesMap::new(count, iter, arena))
            };

            let result = G::new_mut(storage, arena, &mut wtxn)
                .add_edge(label, props, from_id, to_id, false)
                .collect_to_obj()
                .map_err(CompatError::from)?;

            wtxn.commit()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let json = sonic_rs::to_value(&result).unwrap_or_default();
            live_store.insert(bind_to.clone(), vec![result]);
            result_store.insert(bind_to, vec![json]);
        }

        CompatStep::UpdateProperties { seed_var, tool_args, updates, bind_to } => {
            let txn = storage
                .graph_env
                .read_txn()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let targets: Vec<TraversalValue<'arena>> = if let Some(sv) = &seed_var {
                let seeds = live_store.get(sv.as_str()).cloned().unwrap_or_default();
                execute_query_chain_from_seed(&tool_args, storage, &txn, arena, seeds.into_iter())
                    .map_err(CompatError::from)?
                    .collect()
                    .map_err(CompatError::from)?
            } else {
                execute_query_chain(&tool_args, storage, &txn, arena)
                    .map_err(CompatError::from)?
                    .collect()
                    .map_err(CompatError::from)?
            };
            drop(txn);

            let static_updates: Vec<(&'static str, Value)> = updates
                .into_iter()
                .map(|(k, v)| (Box::leak(k.into_boxed_str()) as &'static str, v))
                .collect();

            let mut wtxn = storage
                .graph_env
                .write_txn()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let mut updated = Vec::new();
            for target in targets {
                use crate::sparrow_engine::traversal_core::ops::util::update::UpdateAdapter;
                let node = G::new_mut_from(storage, &mut wtxn, target, arena)
                    .update(&static_updates)
                    .collect_to_obj()
                    .map_err(CompatError::from)?;
                updated.push(node);
            }

            wtxn.commit()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let json_values = serialise_results(&updated);
            live_store.insert(bind_to.clone(), updated);
            result_store.insert(bind_to, json_values);
        }

        CompatStep::DropNodes { seed_var, tool_args } => {
            let txn = storage
                .graph_env
                .read_txn()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            let targets: Vec<TraversalValue<'arena>> = if let Some(sv) = &seed_var {
                let seeds = live_store.get(sv.as_str()).cloned().unwrap_or_default();
                execute_query_chain_from_seed(&tool_args, storage, &txn, arena, seeds.into_iter())
                    .map_err(CompatError::from)?
                    .collect()
                    .map_err(CompatError::from)?
            } else {
                execute_query_chain(&tool_args, storage, &txn, arena)
                    .map_err(CompatError::from)?
                    .collect()
                    .map_err(CompatError::from)?
            };
            drop(txn);

            let mut wtxn = storage
                .graph_env
                .write_txn()
                .map_err(|e| CompatError::Execution(e.to_string()))?;

            for target in &targets {
                match target {
                    TraversalValue::Node(n) => storage
                        .drop_node(&mut wtxn, n.id)
                        .map_err(CompatError::from)?,
                    TraversalValue::Edge(e) => storage
                        .drop_edge(&mut wtxn, e.id)
                        .map_err(CompatError::from)?,
                    _ => {}
                }
            }

            wtxn.commit()
                .map_err(|e| CompatError::Execution(e.to_string()))?;
        }
    }

    Ok(())
}

// ─── response helpers ──────────────────────────────────────────────────────────

/// Serialise a slice of TraversalValues to sonic_rs::Value, adding $id/$label aliases.
fn serialise_results<'arena>(values: &[TraversalValue<'arena>]) -> Vec<sonic_rs::Value> {
    values
        .iter()
        .map(|v| {
            let raw = sonic_rs::to_value(v).unwrap_or_default();
            add_dollar_aliases(raw)
        })
        .collect()
}

/// Add `$id` and `$label` fields as aliases for `id` and `label`.
///
/// Simorgh currently reads `$id` and `$label` from HelixDB responses. We emit both
/// so the transition can happen gradually.
fn add_dollar_aliases(obj: sonic_rs::Value) -> sonic_rs::Value {
    let id_bytes = obj.get("id").and_then(|v| sonic_rs::to_vec(v).ok());
    let label_bytes = obj.get("label").and_then(|v| sonic_rs::to_vec(v).ok());
    if id_bytes.is_none() && label_bytes.is_none() {
        return obj;
    }
    let mut bytes = match sonic_rs::to_vec(&obj) {
        Ok(b) => b,
        Err(_) => return obj,
    };
    if bytes.last() != Some(&b'}') {
        return obj;
    }
    bytes.pop();
    if let Some(id_b) = id_bytes {
        bytes.extend_from_slice(b",\"$id\":");
        bytes.extend_from_slice(&id_b);
    }
    if let Some(label_b) = label_bytes {
        bytes.extend_from_slice(b",\"$label\":");
        bytes.extend_from_slice(&label_b);
    }
    bytes.push(b'}');
    sonic_rs::from_slice(&bytes).unwrap_or_default()
}
