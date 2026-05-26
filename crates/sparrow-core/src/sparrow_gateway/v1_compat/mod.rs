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
    protocol::{request::RequestType, value::Value, Format, Response},
    sparrow_engine::{
        storage_core::{storage_methods::StorageMethods, SparrowGraphStorage},
        traversal_core::traversal_value::TraversalValue,
        types::GraphError,
    },
    sparrow_gateway::{
        gateway::AppState,
        mcp::tools::{
            execute_query_chain, execute_query_chain_from_seed, EdgeType, FilterProperties,
            FilterTraversal, Operator, ToolArgs,
        },
        router::router::{Handler, HandlerInput, HandlerSubmission},
    },
    utils::properties::ImmutablePropertiesMap,
};

use crate::sparrow_engine::bm25::lmdb_bm25::BM25;
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

    // Check the `request_type` field specifically, not a raw byte scan of the whole body.
    let is_write = sonic_rs::from_slice::<sonic_rs::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("request_type")
                .and_then(|t| t.as_str())
                .map(|s| s == "write")
        })
        .unwrap_or(false);
    let handler_name = if is_write {
        "__v1_compat_write"
    } else {
        "__v1_compat_read"
    };

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
                Err(e) => {
                    use crate::sparrow_gateway::auth::TokenError;
                    if !matches!(e, TokenError::InvalidKey | TokenError::Unauthorized) {
                        warn!("v1_compat: token store error during auth: {e}");
                    }
                    sparrow_metrics::log_event(
                        sparrow_metrics::events::EventType::InvalidApiKey,
                        sparrow_metrics::events::InvalidApiKeyEvent {
                            cluster_id: state.cluster_id.clone(),
                            time_taken_usec: 0,
                        },
                    );
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
            info!(
                handler = handler_name,
                elapsed_us = start.elapsed().as_micros(),
                "v1_compat ok"
            );
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

    let query = body
        .get("query")
        .ok_or_else(|| GraphError::DecodeError("v1_compat: missing 'query' field".to_string()))?;

    let raw_queries = query
        .get("queries")
        .and_then(|q| q.as_array())
        .ok_or_else(|| GraphError::DecodeError("v1_compat: missing 'queries' array".to_string()))?;

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
        match result.get(var_name) {
            Some(QueryResult::Count(n)) => {
                // A `"Count"` step was used: return a scalar integer, not a node list.
                let entry = sonic_rs::json!({ "count": n });
                output.insert(var_name.clone(), entry);
            }
            Some(QueryResult::NodeList(nodes)) => {
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
            None => {}
        }
    }

    let body_bytes = sonic_rs::to_vec(&output)
        .map_err(|e| GraphError::DecodeError(format!("v1_compat: serialise: {e}")))?;

    Ok(Response {
        body: body_bytes,
        fmt: Format::Json,
    })
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

/// The result of one named query — either a list of nodes or a scalar count.
enum QueryResult {
    NodeList(Vec<sonic_rs::Value>),
    Count(usize),
}

/// A resolved traversal or mutation step for one named query.
enum CompatStep {
    /// Traverse the graph, optionally seeded from a prior result.
    Traverse {
        seed_var: Option<String>,
        tool_args: Vec<ToolArgs>,
        bind_to: String,
        /// When true, exhaust the iterator and store a scalar count instead of
        /// materialising the full node list.  Set by the `"Count"` step.
        count: bool,
    },
    /// Look up nodes directly by UUID string.
    LookupByUuid { ids: Vec<u128>, bind_to: String },
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
///
/// **Batched transaction**: all write steps (AddN, AddEdge, UpdateProperties,
/// DropNodes) share a single LMDB write transaction that is committed once at
/// the end of the request.  This means:
///
/// * Only **one fsync** is issued per request instead of one per write step,
///   dramatically reducing write latency when a request contains many write
///   operations.
/// * The entire batch is **atomic**: if any step fails the uncommitted
///   transaction is dropped (rolled back) and none of the writes persist.
///
/// Read-only steps (Traverse, LookupByUuid) reuse the write transaction as a
/// read view via `Deref` coercion (`&*wtxn`), allowing them to observe
/// uncommitted writes from earlier steps in the same request.
fn execute_helix_queries<'db, 'arena>(
    raw_queries: &[sonic_rs::Value],
    storage: &'db SparrowGraphStorage,
    arena: &'arena Bump,
) -> Result<HashMap<String, QueryResult>, CompatError>
where
    'db: 'arena,
{
    let mut live_store: HashMap<String, Vec<TraversalValue<'arena>>> = HashMap::new();
    let mut result_store: HashMap<String, QueryResult> = HashMap::new();

    // Open ONE write transaction for the entire request.  All steps — both
    // reads and writes — run inside it.  On success we commit once (one
    // fsync).  On failure the `?` propagates the error and `wtxn` is dropped
    // without committing, rolling back any changes.
    let mut wtxn = storage
        .graph_env
        .write_txn()
        .map_err(|e| CompatError::Execution(e.to_string()))?;

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
            execute_compat_step(
                step,
                storage,
                &mut wtxn,
                arena,
                &mut live_store,
                &mut result_store,
            )?;
        }
    }

    // Commit all writes in a single fsync.
    wtxn.commit()
        .map_err(|e| CompatError::Execution(e.to_string()))?;

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
    // Set to true when a bare `"Count"` step is encountered; honoured at the
    // final flush so that the result is a scalar count, not a node list.
    let mut pending_count = false;

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
                out.push(CompatStep::LookupByUuid {
                    ids: uuids,
                    bind_to: name.to_string(),
                });
                seed_var = Some(name.to_string());
            }
            let args = translate_nwhere(cond)?;
            tool_args.extend(args);
            continue;
        }

        // ── EWhere:{...} ───────────────────────────────────────────────────
        if let Some(cond) = step.get("EWhere") {
            flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
            if let Some(uuids) = pending_uuid_ids.take() {
                out.push(CompatStep::LookupByUuid {
                    ids: uuids,
                    bind_to: name.to_string(),
                });
                seed_var = Some(name.to_string());
            }
            let args = translate_ewhere(cond)?;
            tool_args.extend(args);
            continue;
        }

        // ── Inject:"varname" ───────────────────────────────────────────────
        if let Some(inject_name) = step.get("Inject").and_then(|v| v.as_str()) {
            flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
            if let Some(uuids) = pending_uuid_ids.take() {
                out.push(CompatStep::LookupByUuid {
                    ids: uuids,
                    bind_to: name.to_string(),
                });
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

        // ── OutN:"EDGE" — traverse outgoing edge to destination node ────────
        //
        // OutN is the HelixDB edge-to-node switch that, when used with a label from a
        // node context, behaves identically to Out.  OutN without a label is not
        // well-defined in this context; we log a warning and skip the step.
        if let Some(out_n_val) = step.get("OutN") {
            maybe_flush_uuid_lookup(&mut out, &mut pending_uuid_ids, &mut seed_var, name)?;
            if let Some(edge) = out_n_val.as_str() {
                tool_args.push(ToolArgs::OutStep {
                    edge_label: edge.to_string(),
                    edge_type: EdgeType::Node,
                    filter: None,
                });
            } else {
                warn!("v1_compat: OutN without a string edge label is not supported — use OutN:\"EDGE_LABEL\" or Out:\"EDGE_LABEL\"");
            }
            continue;
        }

        // ── InN:"EDGE" — traverse incoming edge to source node ──────────────
        //
        // InN is the HelixDB edge-to-node switch that, when used with a label from a
        // node context, behaves identically to In.  InN without a label is not
        // well-defined in this context; we log a warning and skip the step.
        if let Some(in_n_val) = step.get("InN") {
            maybe_flush_uuid_lookup(&mut out, &mut pending_uuid_ids, &mut seed_var, name)?;
            if let Some(edge) = in_n_val.as_str() {
                tool_args.push(ToolArgs::InStep {
                    edge_label: edge.to_string(),
                    edge_type: EdgeType::Node,
                    filter: None,
                });
            } else {
                warn!("v1_compat: InN without a string edge label is not supported — use InN:\"EDGE_LABEL\" or In:\"EDGE_LABEL\"");
            }
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
                out.push(CompatStep::LookupByUuid {
                    ids: uuids,
                    bind_to: name.to_string(),
                });
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
            out.push(CompatStep::AddNode {
                node_type,
                fields,
                bind_to: name.to_string(),
            });
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
                .ok_or_else(|| CompatError::Translation("AddE: to.Var is required".to_string()))?
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
            out.push(CompatStep::AddEdge {
                edge_type,
                from_var,
                to_var,
                fields,
                bind_to: name.to_string(),
            });
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
            out.push(CompatStep::DropNodes {
                seed_var: sv,
                tool_args: args,
            });
            return Ok(out);
        }

        // ── VectorSearchNodes:{label, property, query_vector, k} ──────────
        if let Some(vs) = step.get("VectorSearchNodes") {
            flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
            if let Some(uuids) = pending_uuid_ids.take() {
                out.push(CompatStep::LookupByUuid {
                    ids: uuids,
                    bind_to: name.to_string(),
                });
                seed_var = Some(name.to_string());
            }
            let args = translate_vector_search(vs)?;
            tool_args.extend(args);
            continue;
        }

        // ── Count ─────────────────────────────────────────────────────────
        // Mark the pending traversal as a count query.  The final flush will
        // exhaust the iterator and store a scalar integer instead of a node
        // list.  Count is always the last meaningful step; any traversal step
        // that comes after it would start a new sub-query (via flush_traversal)
        // and reset pending_count to false, so it is naturally scoped.
        if step.as_str() == Some("Count") {
            pending_count = true;
            continue;
        }

        // ── Limit: N ──────────────────────────────────────────────────────
        // Truncate the current traversal stream to at most N items.
        if let Some(n_val) = step.get("Limit") {
            maybe_flush_uuid_lookup(&mut out, &mut pending_uuid_ids, &mut seed_var, name)?;
            match n_val.as_u64() {
                Some(n) => tool_args.push(ToolArgs::Limit { n: n as usize }),
                None => warn!("v1_compat: Limit value is not an integer — skipping"),
            }
            continue;
        }

        // ── Id:null, ValueMap:[...], Project:[...] ─────────────────────────
        // Result-shaping hints only — we return all fields and let the caller
        // project what it needs client-side.  No-op in v1_compat.
        if step.get("Id").is_some()
            || step.get("ValueMap").is_some()
            || step.get("Project").is_some()
        {
            continue;
        }

        // ── Unknown step ───────────────────────────────────────────────────
        warn!(step = %step, "v1_compat: unrecognised step — skipping");
    }

    // Flush any accumulated state at end of query.
    if let Some(uuids) = pending_uuid_ids.take() {
        flush_traversal(&mut out, &mut seed_var, &mut tool_args, name)?;
        out.push(CompatStep::LookupByUuid {
            ids: uuids,
            bind_to: name.to_string(),
        });
    } else if !pending_updates.is_empty() {
        let updates: HashMap<String, Value> = pending_updates.into_iter().collect();
        let (sv, args) = (seed_var.take(), std::mem::take(&mut tool_args));
        out.push(CompatStep::UpdateProperties {
            seed_var: sv,
            tool_args: args,
            updates,
            bind_to: name.to_string(),
        });
    } else if seed_var.is_some() || !tool_args.is_empty() {
        // Use pending_count here — the only place where a Count step takes effect.
        out.push(CompatStep::Traverse {
            seed_var: seed_var.take(),
            tool_args: std::mem::take(&mut tool_args),
            bind_to: name.to_string(),
            count: pending_count,
        });
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
            count: false, // intermediate flushes never produce a count
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
        let filter_props: Vec<FilterProperties> =
            filter_props.into_iter().flatten().flatten().collect();
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

/// Translate an `EWhere` condition object into tool args.
///
/// Mirrors `translate_nwhere` but produces `EFromType` (edge source) instead of
/// `NFromType` (node source):
/// `{"Eq": ["$label", {"String": T}]}` → `[EFromType(T)]`
/// `{"And": [{"Eq": ["$label", {"String": T}]}, ...rest...]}` → `[EFromType(T), FilterItems(rest)]`
fn translate_ewhere(cond: &sonic_rs::Value) -> Result<Vec<ToolArgs>, CompatError> {
    if let Some(and_arr) = cond.get("And").and_then(|a| a.as_array()) {
        let label_type = and_arr.iter().find_map(label_from_eq);
        let rest: Vec<&sonic_rs::Value> = and_arr
            .iter()
            .filter(|c| label_from_eq(c).is_none())
            .collect();

        let mut args = Vec::new();
        if let Some(label) = label_type {
            args.push(ToolArgs::EFromType { edge_type: label });
        }
        let filter_props = rest
            .into_iter()
            .map(translate_eq_condition)
            .collect::<Result<Vec<_>, _>>()?;
        let filter_props: Vec<FilterProperties> =
            filter_props.into_iter().flatten().flatten().collect();
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
            return Ok(vec![ToolArgs::EFromType { edge_type: label }]);
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
        "unsupported EWhere condition: {cond}"
    )))
}

/// If this value is `{"Eq": ["$label", {"String": T}]}`, return Some(T).
fn label_from_eq(v: &sonic_rs::Value) -> Option<String> {
    let eq = v.get("Eq")?.as_array()?;
    if eq.len() == 2 && eq[0].as_str() == Some("$label") {
        return eq[1]
            .get("String")
            .and_then(|s| s.as_str())
            .map(str::to_owned);
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
            properties: if props.is_empty() {
                None
            } else {
                Some(vec![props])
            },
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
        ToolArgs::SearchVec {
            vector,
            k,
            min_score: None,
        },
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
        let vals: Vec<Value> = arr
            .iter()
            .map(|f| Value::F64(f.as_f64().unwrap_or(0.0)))
            .collect();
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
    Err(CompatError::Translation(format!(
        "unsupported typed value: {v}"
    )))
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
            let uuid = uuid::Uuid::parse_str(s)
                .map_err(|e| CompatError::Translation(format!("invalid UUID '{s}': {e}")))?;
            ids.push(uuid.as_u128());
        } else if let Some(i) = v.as_u64() {
            // Legacy i64 IDs from HelixDB — treat as raw u128. Likely wrong but don't crash.
            ids.push(i as u128);
        }
    }
    Ok(ids)
}

// ─── execution ─────────────────────────────────────────────────────────────────

fn execute_compat_step<'db, 'txn, 'arena>(
    step: CompatStep,
    storage: &'db SparrowGraphStorage,
    wtxn: &'txn mut heed3::RwTxn<'db>,
    arena: &'arena Bump,
    live_store: &mut HashMap<String, Vec<TraversalValue<'arena>>>,
    result_store: &mut HashMap<String, QueryResult>,
) -> Result<(), CompatError>
where
    'db: 'arena,
    'arena: 'txn,
{
    match step {
        // ── Read-only: use write txn as a read view (sees uncommitted writes
        // from earlier steps in the same request) ────────────────────────────
        CompatStep::Traverse {
            seed_var,
            tool_args,
            bind_to,
            count,
        } => {
            let ro: &heed3::RoTxn<'db> = wtxn;
            if count {
                // Count mode: exhaust the iterator without building a node Vec.
                // This avoids allocating memory proportional to the result set
                // (e.g. 685K book nodes) just to return a single integer.
                let stream = if let Some(sv) = &seed_var {
                    let seeds = live_store.get(sv.as_str()).cloned().unwrap_or_default();
                    execute_query_chain_from_seed(
                        &tool_args,
                        storage,
                        ro,
                        arena,
                        seeds.into_iter(),
                    )
                    .map_err(CompatError::from)?
                } else {
                    execute_query_chain(&tool_args, storage, ro, arena)
                        .map_err(CompatError::from)?
                };
                let mut n: usize = 0;
                for item in stream.into_inner_iter() {
                    item.map_err(CompatError::from)?;
                    n += 1;
                }
                // Count results have no live nodes for downstream steps to seed from.
                live_store.insert(bind_to.clone(), vec![]);
                result_store.insert(bind_to, QueryResult::Count(n));
            } else {
                let values: Vec<TraversalValue<'arena>> = {
                    if let Some(sv) = &seed_var {
                        let seeds = live_store.get(sv.as_str()).cloned().unwrap_or_default();
                        execute_query_chain_from_seed(
                            &tool_args,
                            storage,
                            ro,
                            arena,
                            seeds.into_iter(),
                        )
                        .map_err(CompatError::from)?
                        .collect()
                        .map_err(CompatError::from)?
                    } else {
                        execute_query_chain(&tool_args, storage, ro, arena)
                            .map_err(CompatError::from)?
                            .collect()
                            .map_err(CompatError::from)?
                    }
                };
                let json_values = serialise_results(&values);
                live_store.insert(bind_to.clone(), values);
                result_store.insert(bind_to, QueryResult::NodeList(json_values));
            }
        }

        CompatStep::LookupByUuid { ids, bind_to } => {
            let values: Vec<TraversalValue<'arena>> = {
                let ro: &heed3::RoTxn<'db> = wtxn;
                ids.iter()
                    .filter_map(|&id| {
                        storage
                            .get_node(ro, id, arena)
                            .ok()
                            .map(TraversalValue::Node)
                    })
                    .collect()
            };

            let json_values = serialise_results(&values);
            live_store.insert(bind_to.clone(), values);
            result_store.insert(bind_to, QueryResult::NodeList(json_values));
        }

        // ── Mutations: write directly into the shared transaction ─────────
        CompatStep::AddNode {
            node_type,
            fields,
            bind_to,
        } => {
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
            let iter = fields
                .iter()
                .map(|(k, v)| (arena.alloc_str(k) as &'arena str, v.clone()));
            let props = ImmutablePropertiesMap::new(count, iter, arena);

            let result = G::new_mut(storage, arena, wtxn)
                .add_n(label, Some(props), sec_indices)
                .collect_to_obj()
                .map_err(CompatError::from)?;

            // No commit here — the caller (execute_helix_queries) commits once
            // after all steps succeed.

            let json = sonic_rs::to_value(&result).unwrap_or_default();
            let json_with_aliases = add_dollar_aliases(json);
            live_store.insert(bind_to.clone(), vec![result]);
            result_store.insert(bind_to, QueryResult::NodeList(vec![json_with_aliases]));
        }

        CompatStep::AddEdge {
            edge_type,
            from_var,
            to_var,
            fields,
            bind_to,
        } => {
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

            let label: &'arena str = arena.alloc_str(&edge_type);
            let props = if fields.is_empty() {
                None
            } else {
                let count = fields.len();
                let iter = fields
                    .iter()
                    .map(|(k, v)| (arena.alloc_str(k) as &'arena str, v.clone()));
                Some(ImmutablePropertiesMap::new(count, iter, arena))
            };

            let result = G::new_mut(storage, arena, wtxn)
                .add_edge(label, props, from_id, to_id, false)
                .collect_to_obj()
                .map_err(CompatError::from)?;

            // No commit here.

            let json = sonic_rs::to_value(&result).unwrap_or_default();
            live_store.insert(bind_to.clone(), vec![result]);
            result_store.insert(bind_to, QueryResult::NodeList(vec![json]));
        }

        CompatStep::UpdateProperties {
            seed_var,
            tool_args,
            updates,
            bind_to,
        } => {
            // Read phase: collect targets using the write txn as a read view.
            // The block scope ensures the immutable borrow of `wtxn` ends
            // before the mutable borrow in the write phase below.
            let targets: Vec<TraversalValue<'arena>> = {
                let ro: &heed3::RoTxn<'db> = wtxn;
                if let Some(sv) = &seed_var {
                    let seeds = live_store.get(sv.as_str()).cloned().unwrap_or_default();
                    execute_query_chain_from_seed(&tool_args, storage, ro, arena, seeds.into_iter())
                        .map_err(CompatError::from)?
                        .collect()
                        .map_err(CompatError::from)?
                } else {
                    execute_query_chain(&tool_args, storage, ro, arena)
                        .map_err(CompatError::from)?
                        .collect()
                        .map_err(CompatError::from)?
                }
                // `ro` borrow released here
            };

            let static_updates: Vec<(&'static str, Value)> = updates
                .into_iter()
                .map(|(k, v)| (Box::leak(k.into_boxed_str()) as &'static str, v))
                .collect();

            // Write phase: apply updates through the shared write transaction.
            let mut updated = Vec::new();
            for target in targets {
                use crate::sparrow_engine::traversal_core::ops::util::update::UpdateAdapter;
                let node = G::new_mut_from(storage, wtxn, target, arena)
                    .update(&static_updates)
                    .collect_to_obj()
                    .map_err(CompatError::from)?;
                updated.push(node);
            }

            // No commit here.

            let json_values = serialise_results(&updated);
            live_store.insert(bind_to.clone(), updated);
            result_store.insert(bind_to, QueryResult::NodeList(json_values));
        }

        CompatStep::DropNodes {
            seed_var,
            tool_args,
        } => {
            // Read phase: find nodes/edges to drop.
            let targets: Vec<TraversalValue<'arena>> = {
                let ro: &heed3::RoTxn<'db> = wtxn;
                if let Some(sv) = &seed_var {
                    let seeds = live_store.get(sv.as_str()).cloned().unwrap_or_default();
                    execute_query_chain_from_seed(&tool_args, storage, ro, arena, seeds.into_iter())
                        .map_err(CompatError::from)?
                        .collect()
                        .map_err(CompatError::from)?
                } else {
                    execute_query_chain(&tool_args, storage, ro, arena)
                        .map_err(CompatError::from)?
                        .collect()
                        .map_err(CompatError::from)?
                }
                // `ro` borrow released here
            };

            // Write phase: delete through the shared write transaction.
            for target in &targets {
                match target {
                    TraversalValue::Node(n) => {
                        storage.drop_node(wtxn, n.id).map_err(CompatError::from)?;
                        // Also remove the BM25 document so stale terms don't
                        // inflate scores or return ghost results.  Mirror the
                        // same guard used in drop.rs::Drop::drop_traversal.
                        if let Some(bm25) = storage.bm25.as_ref().filter(|_| {
                            !storage
                                .skip_bm25_writes
                                .load(std::sync::atomic::Ordering::Acquire)
                                && !storage.bm25_exclude_labels.contains(n.label)
                        }) {
                            if let Err(e) = bm25.delete_doc(wtxn, n.id) {
                                warn!(
                                    node_id = ?n.id,
                                    "v1_compat drop: BM25 deletion failed: {e}"
                                );
                            }
                        }
                    }
                    TraversalValue::Edge(e) => {
                        storage.drop_edge(wtxn, e.id).map_err(CompatError::from)?
                    }
                    _ => {}
                }
            }

            // No commit here.
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

// ─── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(all(feature = "lmdb", feature = "server"))]
mod tests {
    use super::v1_compat_handler;
    use crate::{
        protocol::{request::RequestType, Format, Request},
        sparrow_engine::traversal_core::{
            config::Config, SparrowGraphEngine, SparrowGraphEngineOpts,
        },
        sparrow_gateway::router::router::HandlerInput,
    };
    use axum::body::Bytes;
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_test_graph() -> (Arc<SparrowGraphEngine>, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.db_max_size_gb = Some(0);
        let opts = SparrowGraphEngineOpts {
            path: dir.path().to_str().unwrap().to_string(),
            config,
            version_info: Default::default(),
            skip_bm25_on_write: None,
        };
        let graph = Arc::new(SparrowGraphEngine::new(opts).unwrap());
        (graph, dir)
    }

    fn make_write_request(body: impl Into<Bytes>) -> Request {
        Request {
            name: "__v1_compat_write".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            body: body.into(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
            pre_computed_embedding: None,
        }
    }

    fn make_read_request(body: impl Into<Bytes>) -> Request {
        Request {
            name: "__v1_compat_read".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            body: body.into(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
            pre_computed_embedding: None,
        }
    }

    fn node_count(graph: &SparrowGraphEngine) -> u64 {
        let txn = graph.storage.graph_env.read_txn().unwrap();
        graph.storage.nodes_db.len(&txn).unwrap_or(0)
    }

    /// Add a node with the given label and return its UUID string.
    fn add_node(graph: &Arc<SparrowGraphEngine>, label: &str) -> String {
        let body = format!(
            r#"{{"request_type":"write","query":{{"queries":[{{"Query":{{"name":"n","steps":[{{"AddN":{{"label":"{label}","properties":[["_k",{{"Value":{{"String":"{label}"}}}}]]}}}}]}}}}],"returns":["n"]}}}}"#
        );
        let resp = v1_compat_handler(HandlerInput {
            request: make_write_request(body),
            graph: graph.clone(),
        })
        .expect("add_node must succeed");
        let json: sonic_rs::Value = sonic_rs::from_slice(&resp.body).expect("valid JSON response");
        json.get("n")
            .and_then(|n| n.get("ids"))
            .and_then(|ids| ids.as_array())
            .and_then(|arr| arr.first())
            .and_then(|id| id.as_str())
            .expect("n.ids[0] must be a UUID string")
            .to_string()
    }

    /// Create an edge from_id --label--> to_id.
    fn add_edge(graph: &Arc<SparrowGraphEngine>, from_id: &str, to_id: &str, label: &str) {
        let body = format!(
            r#"{{
            "request_type": "write",
            "query": {{
                "queries": [
                    {{"Query": {{"name": "src", "steps": [{{"N": {{"Ids": ["{from_id}"]}}}}]}}}},
                    {{"Query": {{"name": "tgt", "steps": [{{"N": {{"Ids": ["{to_id}"]}}}}]}}}},
                    {{"Query": {{"name": "e", "steps": [
                        {{"Inject": "src"}},
                        {{"AddE": {{"label": "{label}", "to": {{"Var": "tgt"}}, "properties": []}}}}
                    ]}}}}
                ],
                "returns": ["e"]
            }}
        }}"#
        );
        v1_compat_handler(HandlerInput {
            request: make_write_request(body),
            graph: graph.clone(),
        })
        .expect("add_edge must succeed");
    }

    /// A request with N AddN steps followed by a step that fails must commit
    /// nothing — all writes in a single request are one atomic unit.
    ///
    /// RED: With the current implementation each AddN opens its own write_txn
    /// and commits immediately, so the first node persists even when a later
    /// step fails.  This test must FAIL before the batch-transaction fix is
    /// applied.
    #[test]
    #[serial_test::serial]
    fn batch_write_is_atomic_rollback_on_failure() {
        let (graph, _dir) = make_test_graph();
        assert_eq!(node_count(&graph), 0, "precondition: DB is empty");

        // Request:
        //   1. AddN a "zettel_note" node → this commits its own txn with current code
        //   2. AddE referencing a variable "ghost" that was never bound → always fails
        //
        // Expected after fix: 0 nodes (whole request rolled back atomically).
        // Actual with current code: 1 node (AddN committed before AddE failed).
        let body = r#"{
            "request_type": "write",
            "query": {
                "queries": [
                    {"Query": {"name": "note", "steps": [
                        {"AddN": {"label": "zettel_note", "properties": [
                            ["title", {"Value": {"String": "atomic test"}}]
                        ]}}
                    ]}},
                    {"Query": {"name": "bad_edge", "steps": [
                        {"Inject": "note"},
                        {"AddE": {"label": "LINKS_TO", "to": {"Var": "ghost_var_never_bound"}}}
                    ]}}
                ],
                "returns": ["note"]
            }
        }"#;

        let input = HandlerInput {
            request: make_write_request(body),
            graph: graph.clone(),
        };

        let result = v1_compat_handler(input);
        assert!(
            result.is_err(),
            "handler must return an error when AddE references an unbound variable"
        );

        assert_eq!(
            node_count(&graph),
            0,
            "batch write must be atomic: the AddN node must not persist when a later step in the same request fails"
        );
    }

    /// Multiple AddN steps in one request must all be visible after success.
    /// This verifies the happy path of the batched-transaction implementation.
    #[test]
    #[serial_test::serial]
    fn batch_write_commits_all_nodes_on_success() {
        let (graph, _dir) = make_test_graph();
        assert_eq!(node_count(&graph), 0, "precondition: DB is empty");

        let body = r#"{
            "request_type": "write",
            "query": {
                "queries": [
                    {"Query": {"name": "n1", "steps": [
                        {"AddN": {"label": "zettel_note", "properties": [["title", {"Value": {"String": "first"}}]]}}
                    ]}},
                    {"Query": {"name": "n2", "steps": [
                        {"AddN": {"label": "zettel_note", "properties": [["title", {"Value": {"String": "second"}}]]}}
                    ]}},
                    {"Query": {"name": "n3", "steps": [
                        {"AddN": {"label": "zettel_note", "properties": [["title", {"Value": {"String": "third"}}]]}}
                    ]}}
                ],
                "returns": ["n1", "n2", "n3"]
            }
        }"#;

        let input = HandlerInput {
            request: make_write_request(body),
            graph: graph.clone(),
        };

        let result = v1_compat_handler(input);
        assert!(result.is_ok(), "three AddN in one request should succeed");
        assert_eq!(node_count(&graph), 3, "all three nodes must be persisted");
    }

    // ── Bug-fix tests: EWhere and OutN/InN ───────────────────────────────────────

    /// EWhere must return all edges whose `$label` matches the predicate.
    ///
    /// Bug A (before fix): EWhere is silently skipped → result_store has no entry for
    /// the query name → response is `{}` (empty object, "r" key absent).
    #[test]
    #[serial_test::serial]
    fn ewhere_returns_edges_matching_label() {
        let (graph, _dir) = make_test_graph();
        let a_id = add_node(&graph, "TestA");
        let b_id = add_node(&graph, "TestB");
        add_edge(&graph, &a_id, &b_id, "LINKS_TO");

        let body = r#"{
            "request_type": "read",
            "query": {
                "queries": [{"Query": {"name": "r", "steps": [
                    {"EWhere": {"Eq": ["$label", {"String": "LINKS_TO"}]}}
                ]}}],
                "returns": ["r"]
            }
        }"#;
        let resp = v1_compat_handler(HandlerInput {
            request: make_read_request(body),
            graph: graph.clone(),
        })
        .expect("EWhere query must not return an error");

        let json: sonic_rs::Value =
            sonic_rs::from_slice(&resp.body).expect("response must be valid JSON");

        // Bug A: before the fix the response was `{}` — the "r" key was absent entirely.
        assert!(
            json.get("r").is_some(),
            "EWhere response must include the 'r' result key; got: {}",
            sonic_rs::to_string(&json).unwrap_or_default()
        );
        let ids = json
            .get("r")
            .and_then(|r| r.get("ids"))
            .and_then(|ids| ids.as_array())
            .expect("r.ids must be an array");
        assert_eq!(
            ids.len(),
            1,
            "exactly one LINKS_TO edge must be found; got {} ids",
            ids.len()
        );
    }

    /// OutN with a string edge label must traverse to the destination node.
    ///
    /// Bug B (before fix): OutN is silently skipped → pending_uuid_ids for the
    /// preceding N:{Ids:[...]} step is flushed at end-of-query → starting node
    /// returned unchanged (TestA instead of TestB).
    #[test]
    #[serial_test::serial]
    fn outn_traverses_to_destination_node() {
        let (graph, _dir) = make_test_graph();
        let a_id = add_node(&graph, "TestA");
        let b_id = add_node(&graph, "TestB");
        add_edge(&graph, &a_id, &b_id, "LINKS_TO");

        let body = format!(
            r#"{{
            "request_type": "read",
            "query": {{
                "queries": [{{"Query": {{"name": "r", "steps": [
                    {{"N": {{"Ids": ["{a_id}"]}}}},
                    {{"OutN": "LINKS_TO"}}
                ]}}}}],
                "returns": ["r"]
            }}
        }}"#
        );
        let resp = v1_compat_handler(HandlerInput {
            request: make_read_request(body),
            graph: graph.clone(),
        })
        .expect("OutN query must not return an error");

        let json: sonic_rs::Value =
            sonic_rs::from_slice(&resp.body).expect("response must be valid JSON");

        let label = json
            .get("r")
            .and_then(|r| r.get("properties"))
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|n| n.get("label"))
            .and_then(|l| l.as_str())
            .expect("r.properties[0].label must be present");

        assert_eq!(
            label, "TestB",
            "OutN must return the destination node (TestB); got '{label}' — Bug B: starting node returned unchanged"
        );
    }

    /// InN with a string edge label must traverse to the source node.
    ///
    /// Bug B (before fix): InN is silently skipped → starting node returned unchanged
    /// (TestB instead of TestA).
    #[test]
    #[serial_test::serial]
    fn inn_traverses_to_source_node() {
        let (graph, _dir) = make_test_graph();
        let a_id = add_node(&graph, "TestA");
        let b_id = add_node(&graph, "TestB");
        add_edge(&graph, &a_id, &b_id, "LINKS_TO");

        let body = format!(
            r#"{{
            "request_type": "read",
            "query": {{
                "queries": [{{"Query": {{"name": "r", "steps": [
                    {{"N": {{"Ids": ["{b_id}"]}}}},
                    {{"InN": "LINKS_TO"}}
                ]}}}}],
                "returns": ["r"]
            }}
        }}"#
        );
        let resp = v1_compat_handler(HandlerInput {
            request: make_read_request(body),
            graph: graph.clone(),
        })
        .expect("InN query must not return an error");

        let json: sonic_rs::Value =
            sonic_rs::from_slice(&resp.body).expect("response must be valid JSON");

        let label = json
            .get("r")
            .and_then(|r| r.get("properties"))
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|n| n.get("label"))
            .and_then(|l| l.as_str())
            .expect("r.properties[0].label must be present");

        assert_eq!(
            label, "TestA",
            "InN must return the source node (TestA); got '{label}' — Bug B: starting node returned unchanged"
        );
    }

    // ── Bug-fix tests: Out + Where {Eq: [$id, ...]} false negative ──────────────

    /// `N(ids) → Out(label) → Where {Eq: ["$id", target]} → Id` must return the
    /// target node's id when the edge exists — this is the canonical `ensure_edge`
    /// check-then-act pattern used in simorgh.
    ///
    /// Root cause (before fix): `Where` evaluated `$id` via `get_property("id")`
    /// which only looks in the stored property bag.  The node `id` field is a
    /// struct-level `u128`, not a stored property, so `get_property("id")` always
    /// returned `None` → `unwrap_or(false)` → every node filtered out → empty result.
    ///
    /// Fix: `matches_properties` in `tools.rs` now handles the special keys `"id"`
    /// and `"label"` by reading the struct-level fields directly and comparing the
    /// filter UUID string against `item.id()`.
    #[test]
    #[serial_test::serial]
    fn out_where_eq_id_returns_destination_node_when_edge_exists() {
        let (graph, _dir) = make_test_graph();
        let a_id = add_node(&graph, "Source");
        let b_id = add_node(&graph, "Destination");
        add_edge(&graph, &a_id, &b_id, "MEMBER_OF");

        // This is the exact pattern used by simorgh's ensure_edge check:
        // traverse from `a` via MEMBER_OF, then filter for nodes whose $id == b_id.
        let body = format!(
            r#"{{
            "request_type": "read",
            "query": {{
                "queries": [{{"Query": {{"name": "exists", "steps": [
                    {{"N": {{"Ids": ["{a_id}"]}}}},
                    {{"Out": "MEMBER_OF"}},
                    {{"Where": {{"Eq": ["$id", {{"String": "{b_id}"}}]}}}},
                    {{"Id": null}}
                ], "condition": null}}}}],
                "returns": ["exists"]
            }}
        }}"#
        );

        let resp = v1_compat_handler(HandlerInput {
            request: make_read_request(body),
            graph: graph.clone(),
        })
        .expect("Out+Where query must not return an error");

        let json: sonic_rs::Value =
            sonic_rs::from_slice(&resp.body).expect("response must be valid JSON");

        let ids = json
            .get("exists")
            .and_then(|e| e.get("ids"))
            .and_then(|ids| ids.as_array())
            .expect("exists.ids must be an array");

        assert_eq!(
            ids.len(),
            1,
            "Out+Where must find the destination node when the edge exists; got {} ids — \
             false-negative bug: $id comparison against struct-level id field was broken",
            ids.len()
        );
        assert_eq!(
            ids[0].as_str().unwrap_or(""),
            b_id,
            "the returned id must be the destination node's id (b_id)"
        );
    }

    /// Complement test: `Out + Where {Eq: ["$id", wrong_id]}` must return empty
    /// when no edge connects to that target — i.e., the fix must not over-match.
    #[test]
    #[serial_test::serial]
    fn out_where_eq_id_returns_empty_when_no_matching_edge() {
        let (graph, _dir) = make_test_graph();
        let a_id = add_node(&graph, "Source");
        let b_id = add_node(&graph, "Destination");
        let c_id = add_node(&graph, "Unrelated");
        add_edge(&graph, &a_id, &b_id, "MEMBER_OF");

        // Query for c_id (not connected to a via MEMBER_OF) — must return empty.
        let body = format!(
            r#"{{
            "request_type": "read",
            "query": {{
                "queries": [{{"Query": {{"name": "exists", "steps": [
                    {{"N": {{"Ids": ["{a_id}"]}}}},
                    {{"Out": "MEMBER_OF"}},
                    {{"Where": {{"Eq": ["$id", {{"String": "{c_id}"}}]}}}},
                    {{"Id": null}}
                ], "condition": null}}}}],
                "returns": ["exists"]
            }}
        }}"#
        );

        let resp = v1_compat_handler(HandlerInput {
            request: make_read_request(body),
            graph: graph.clone(),
        })
        .expect("Out+Where query must not return an error");

        let json: sonic_rs::Value =
            sonic_rs::from_slice(&resp.body).expect("response must be valid JSON");

        let ids = json
            .get("exists")
            .and_then(|e| e.get("ids"))
            .and_then(|ids| ids.as_array())
            .expect("exists.ids must be an array");

        assert_eq!(
            ids.len(),
            0,
            "Out+Where must return empty when no edge to the target exists; got {} ids",
            ids.len()
        );
    }

    // ── Bug-fix tests: Count and Limit ───────────────────────────────────────────

    /// `"Count"` as a bare step must return `{"total": {"count": N}}`, not the full
    /// node list.
    ///
    /// Bug (before fix): Count was a no-op — the same full NWhere scan was returned.
    #[test]
    #[serial_test::serial]
    fn count_step_returns_integer_not_full_node_list() {
        let (graph, _dir) = make_test_graph();
        add_node(&graph, "Widget");
        add_node(&graph, "Widget");
        add_node(&graph, "Widget");

        let body = r#"{
            "request_type": "read",
            "query": {
                "queries": [{"Query": {"name": "total", "steps": [
                    {"NWhere": {"Eq": ["$label", {"String": "Widget"}]}},
                    "Count"
                ], "condition": null}}],
                "returns": ["total"]
            }
        }"#;

        let resp = v1_compat_handler(HandlerInput {
            request: make_read_request(body),
            graph: graph.clone(),
        })
        .expect("Count query must not return an error");

        let json: sonic_rs::Value =
            sonic_rs::from_slice(&resp.body).expect("response must be valid JSON");

        // Bug: before the fix, total.count was absent and total.ids had 3 elements.
        let count = json
            .get("total")
            .and_then(|t| t.get("count"))
            .and_then(|c| c.as_u64())
            .expect("total.count must be an integer — got missing or wrong shape");

        assert_eq!(count, 3, "Count must return 3 (one per Widget node)");

        // Also assert ids/properties are NOT present — this is a count result, not a node list.
        assert!(
            json.get("total").and_then(|t| t.get("ids")).is_none(),
            "Count result must not include an 'ids' array"
        );
    }

    /// `{"Limit": N}` must truncate the result set to N items.
    ///
    /// Bug (before fix): Limit was an unrecognised step and was silently skipped —
    /// all items were returned regardless of N.
    #[test]
    #[serial_test::serial]
    fn limit_step_truncates_result_to_n_items() {
        let (graph, _dir) = make_test_graph();
        for _ in 0..5 {
            add_node(&graph, "Gadget");
        }

        let body = r#"{
            "request_type": "read",
            "query": {
                "queries": [{"Query": {"name": "r", "steps": [
                    {"NWhere": {"Eq": ["$label", {"String": "Gadget"}]}},
                    {"Limit": 3}
                ], "condition": null}}],
                "returns": ["r"]
            }
        }"#;

        let resp = v1_compat_handler(HandlerInput {
            request: make_read_request(body),
            graph: graph.clone(),
        })
        .expect("Limit query must not return an error");

        let json: sonic_rs::Value =
            sonic_rs::from_slice(&resp.body).expect("response must be valid JSON");

        let ids = json
            .get("r")
            .and_then(|r| r.get("ids"))
            .and_then(|ids| ids.as_array())
            .expect("r.ids must be an array");

        // Bug: before the fix, all 5 items were returned because Limit was a no-op.
        assert_eq!(
            ids.len(),
            3,
            "Limit 3 must return exactly 3 items (got {}); Limit was a no-op before fix",
            ids.len()
        );
    }
}
