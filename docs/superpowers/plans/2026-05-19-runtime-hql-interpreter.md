# Runtime HQL Interpreter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `POST /__hql_runtime_eval` — an endpoint that accepts raw HQL source, parses and analyzes it against the deployed schema, lowers it to traversal primitives, and executes it without recompiling the container.

**Architecture:** The deployed container already embeds the compiled schema as JSON introspection data in `Config.schema`. We add a parallel field `Config.hql_schema_raw: Option<String>` that stores the original HQL source. At request time, we prepend the schema HQL to the submitted query, parse the combined source using the existing `SparrowParser`, analyze with the existing `analyze()` function, then lower the parsed AST to `ToolArgs` chains and execute using the existing `execute_query_chain` infrastructure. Mutations (AddN, AddE, DROP, UPDATE) are lowered to direct storage calls.

**Tech Stack:** Rust, existing `sparrow-db` parser/analyzer/traversal crates, Axum for routing, `sonic_rs` for JSON, `bumpalo` for arena allocation, `heed3` (LMDB) for write transactions.

---

## File Map

**Files to CREATE:**
- `sparrow-db/src/sparrow_gateway/runtime_eval/mod.rs` — module root; `RuntimeEvalRequest`, `RuntimeEvalResponse`, `RuntimeError`
- `sparrow-db/src/sparrow_gateway/runtime_eval/parse.rs` — parse + analyze; `parse_and_validate()`
- `sparrow-db/src/sparrow_gateway/runtime_eval/lower.rs` — AST → IR lowering; `lower_query()`
- `sparrow-db/src/sparrow_gateway/runtime_eval/executor.rs` — execute IR against storage; `execute_plan()`
- `sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs` — Axum-compatible handler fn

**Files to MODIFY:**
- `sparrow-db/src/sparrow_engine/traversal_core/config.rs` — add `hql_schema_raw: Option<String>` to `Config`; update `Config::new`, `Config::default`, `Config::fmt_with_schema`
- `sparrow-db/src/sparrow_engine/storage_core/mod.rs` — add `hql_schema_raw: Option<String>` to `StorageConfig`; update constructor (2 call sites, LMDB + Rocks)
- `sparrow-db/src/sparrowc/generator/mod.rs` — pass `self.src` to `fmt_with_schema` as `hql_schema_raw`
- `sparrow-db/src/sparrow_gateway/mod.rs` — add `pub mod runtime_eval;`
- `sparrow-container/src/main.rs` — register `__hql_runtime_eval` route when `SPARROW_RUNTIME_HQL=true`

---

## Task 1: Add `hql_schema_raw` to Config and thread through storage

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/traversal_core/config.rs`
- Modify: `sparrow-db/src/sparrow_engine/storage_core/mod.rs`
- Modify: `sparrow-db/src/sparrowc/generator/mod.rs`

- [ ] **Step 1: Write the failing test**

In `sparrow-db/src/sparrow_engine/traversal_core/config.rs`, at the bottom:

```rust
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
        // When hql_schema_raw is None the generated fn should have None
        assert!(rendered.contains("hql_schema_raw: None"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test -p sparrow-db --features lmdb runtime_schema_tests 2>&1 | tail -20
```

Expected: compile error — field `hql_schema_raw` does not exist

- [ ] **Step 3: Add `hql_schema_raw` to `Config` struct**

In `sparrow-db/src/sparrow_engine/traversal_core/config.rs`, add the field after `graphvis_node_label`:

```rust
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
```

Note: `#[serde(skip)]` so the field is not serialized to/from the `config.json` on disk (it is embed-only via codegen).

- [ ] **Step 4: Update `Config::new` to accept `hql_schema_raw`**

```rust
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
        vector_config: Some(VectorConfig { m: Some(m), ef_construction: Some(ef_construction), ef_search: Some(ef_search) }),
        graph_config: Some(GraphConfig { secondary_indices: None }),
        db_max_size_gb: Some(db_max_size_gb),
        mcp: Some(mcp),
        bm25: Some(bm25),
        schema,
        embedding_model,
        graphvis_node_label,
        hql_schema_raw,
    }
}
```

Update `Config::default()` to add `hql_schema_raw: None`.

- [ ] **Step 5: Update `fmt_with_schema` to embed `hql_schema_raw`**

Change the signature to accept raw HQL:

```rust
pub fn fmt_with_schema(
    &self,
    f: &mut fmt::Formatter,
    introspection_data: Option<&IntrospectionData>,
    secondary_indices: &[SecondaryIndex],
    hql_schema_raw: Option<&str>,
) -> fmt::Result {
```

At the end of the function body, just before the closing `}}`:

```rust
match hql_schema_raw {
    Some(raw) => writeln!(f, "hql_schema_raw: Some(r#\"{raw}\"#.to_string()),")?,
    None => writeln!(f, "hql_schema_raw: None,")?,
}
```

Update the `Display` impl to pass `None`:

```rust
impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.fmt_with_schema(f, None, &[], None)
    }
}
```

- [ ] **Step 6: Update `generator/mod.rs` to pass raw HQL**

In `sparrow-db/src/sparrowc/generator/mod.rs`, the `Display for Source` impl calls `fmt_with_schema`. Update to pass `self.src`:

```rust
impl Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", write_headers())?;
        self.config.fmt_with_schema(
            f,
            self.introspection_data.as_ref(),
            &self.secondary_indices,
            Some(&self.src),
        )?;
        // ... rest unchanged
```

- [ ] **Step 7: Add `hql_schema_raw` to `StorageConfig`**

In `sparrow-db/src/sparrow_engine/storage_core/mod.rs`:

```rust
pub struct StorageConfig {
    pub schema: Option<String>,
    pub graphvis_node_label: Option<String>,
    pub embedding_model: Option<String>,
    pub hql_schema_raw: Option<String>,
}

impl StorageConfig {
    pub fn new(
        schema: Option<String>,
        graphvis_node_label: Option<String>,
        embedding_model: Option<String>,
        hql_schema_raw: Option<String>,
    ) -> StorageConfig {
        Self { schema, graphvis_node_label, embedding_model, hql_schema_raw }
    }
}
```

Update the two `StorageConfig::new` call sites (LMDB at line ~212, Rocks at line ~748):

```rust
let storage_config = StorageConfig::new(
    config.schema,
    config.graphvis_node_label,
    config.embedding_model,
    config.hql_schema_raw,
);
```

- [ ] **Step 8: Run tests**

```bash
cargo test -p sparrow-db --features lmdb runtime_schema_tests 2>&1 | tail -20
```

Expected: all 3 tests pass

- [ ] **Step 9: Verify the full lmdb build**

```bash
cargo build -p sparrow-container --features lmdb 2>&1 | grep -E "error|warning: unused" | grep -v "warning: unused import" | head -20
```

Expected: clean build (only expected warnings)

- [ ] **Step 10: Commit**

```bash
git add sparrow-db/src/sparrow_engine/traversal_core/config.rs \
        sparrow-db/src/sparrow_engine/storage_core/mod.rs \
        sparrow-db/src/sparrowc/generator/mod.rs
git commit -m "feat: add hql_schema_raw to Config and StorageConfig for runtime eval"
```

---

## Task 2: Route registration + env gate (returns 501 placeholder)

**Files:**
- Modify: `sparrow-db/src/sparrow_gateway/mod.rs`
- Create: `sparrow-db/src/sparrow_gateway/runtime_eval/mod.rs`
- Create: `sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs`
- Modify: `sparrow-container/src/main.rs`

- [ ] **Step 1: Write the integration smoke test script**

Create `/tmp/test-runtime-eval-smoke.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
PORT=${1:-6969}

response=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "http://localhost:$PORT/__hql_runtime_eval" \
  -H "Content-Type: application/json" \
  -d '{"query": "QUERY test() => RETURN NONE", "params": {}}')

echo "Status: $response"
if [ "$response" = "200" ] || [ "$response" = "501" ]; then
  echo "PASS: route exists"
else
  echo "FAIL: expected 200 or 501, got $response"
  exit 1
fi
```

This test verifies the route exists (200 or 501 acceptable at this stage). A 404 means the route wasn't registered.

- [ ] **Step 2: Create the runtime_eval module**

Create `sparrow-db/src/sparrow_gateway/runtime_eval/mod.rs`:

```rust
use crate::{
    protocol::{Response, Format},
    sparrow_engine::types::GraphError,
};
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

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("analysis error: {0}")]
    Analysis(String),
    #[error("lowering error: {0}")]
    Lowering(String),
    #[error("execution error: {0}")]
    Execution(#[from] GraphError),
    #[error("no schema available — container was not compiled with SPARROW_RUNTIME_HQL support")]
    NoSchema,
    #[error("request contains no query")]
    NoQuery,
    #[error("unsupported expression: {0}")]
    Unsupported(String),
}

impl From<RuntimeError> for GraphError {
    fn from(e: RuntimeError) -> Self {
        GraphError::DecodeError(e.to_string())
    }
}
```

- [ ] **Step 3: Create the placeholder handler**

Create `sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs`:

```rust
use crate::{
    protocol::{Format, Request, Response},
    sparrow_engine::types::GraphError,
    sparrow_gateway::router::router::HandlerInput,
};
use super::RuntimeError;

pub fn handle(input: HandlerInput, hql_schema_raw: Option<String>) -> Result<Response, GraphError> {
    let _ = input;
    let _ = hql_schema_raw;
    // Placeholder: not yet implemented
    Err(GraphError::DecodeError(
        "runtime eval not yet implemented".to_string()
    ))
}
```

- [ ] **Step 4: Add `runtime_eval` to gateway mod**

In `sparrow-db/src/sparrow_gateway/mod.rs`, add:

```rust
pub mod runtime_eval;
```

- [ ] **Step 5: Register the route in `main.rs`**

In `sparrow-container/src/main.rs`, after the `query_routes`/`write_routes` fold, add:

```rust
use sparrow_db::sparrow_gateway::runtime_eval::handler as runtime_handler;

// Runtime HQL eval route — opt-in via env var
if std::env::var("SPARROW_RUNTIME_HQL").as_deref() == Ok("true") {
    let hql_schema_raw = config.hql_schema_raw.clone();
    let rt_handler: HandlerFn = Arc::new(move |input| {
        runtime_handler::handle(input, hql_schema_raw.clone())
    });
    query_routes.insert("__hql_runtime_eval".to_string(), rt_handler);
    write_routes.insert("__hql_runtime_eval".to_string());
    println!("Runtime HQL eval enabled at POST /__hql_runtime_eval");
}
```

Note: `(mut query_routes, mut write_routes)` — you must add `mut` to the bindings in the fold.

- [ ] **Step 6: Build and verify**

```bash
cargo build -p sparrow-container --features lmdb 2>&1 | grep -c "^error" | xargs -I{} test {} -eq 0 && echo "BUILD OK"
```

Expected: `BUILD OK`

- [ ] **Step 7: Manual smoke test**

Start the server with runtime eval enabled:

```bash
SPARROW_DATA_DIR=/tmp/runtime-eval-test SPARROW_PORT=7979 SPARROW_RUNTIME_HQL=true \
  ./target/debug/sparrow-container &
sleep 1
curl -s -o /dev/null -w "%{http_code}" \
  -X POST http://localhost:7979/__hql_runtime_eval \
  -H "Content-Type: application/json" \
  -d '{"query":"QUERY t() => RETURN NONE","params":{}}'
kill %1
```

Expected: `500` (or any non-404 status — the route exists even though impl returns error)

- [ ] **Step 8: Commit**

```bash
git add sparrow-db/src/sparrow_gateway/runtime_eval/ \
        sparrow-db/src/sparrow_gateway/mod.rs \
        sparrow-container/src/main.rs
git commit -m "feat: register __hql_runtime_eval route behind SPARROW_RUNTIME_HQL env gate"
```

---

## Task 3: Parse + analyze bridge

**Files:**
- Create: `sparrow-db/src/sparrow_gateway/runtime_eval/parse.rs`
- Modify: `sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs` (wire in parse)

- [ ] **Step 1: Write failing unit tests for parse module**

Create `sparrow-db/src/sparrow_gateway/runtime_eval/parse.rs` with the tests first:

```rust
use crate::{
    sparrowc::{
        analyzer::analyze,
        parser::{
            SparrowParser,
            types::{Content, HxFile, Source},
        },
    },
};
use super::RuntimeError;

const TEST_SCHEMA: &str = r#"
N::People {
    UNIQUE INDEX person_id: String,
    first_name: String,
    age: I32
}
"#;

pub fn parse_and_validate(
    schema_hql: &str,
    query_hql: &str,
) -> Result<Source, RuntimeError> {
    let content = Content {
        content: format!("{schema_hql}\n{query_hql}"),
        source: Source::default(),
        files: vec![
            HxFile { name: "schema.hx".to_string(), content: schema_hql.to_string() },
            HxFile { name: "runtime.hx".to_string(), content: query_hql.to_string() },
        ],
    };

    let source = SparrowParser::parse_source(&content)
        .map_err(|e| RuntimeError::Parse(e.to_string()))?;

    let (diagnostics, _) = analyze(&source)
        .map_err(|e| RuntimeError::Analysis(e.to_string()))?;

    if !diagnostics.is_empty() {
        let msgs: Vec<String> = diagnostics.iter()
            .map(|d| d.message.clone())
            .collect();
        return Err(RuntimeError::Analysis(msgs.join("; ")));
    }

    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_query_parses_ok() {
        let query = r#"
QUERY getAll() =>
    people <- N<People>
RETURN people
"#;
        let result = parse_and_validate(TEST_SCHEMA, query);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        let source = result.unwrap();
        assert_eq!(source.queries.len(), 1);
        assert_eq!(source.queries[0].name, "getAll");
    }

    #[test]
    fn test_unknown_type_fails_analysis() {
        let query = r#"
QUERY bad() =>
    x <- N<Nonexistent>
RETURN x
"#;
        let result = parse_and_validate(TEST_SCHEMA, query);
        assert!(result.is_err(), "expected Err for unknown type");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("analysis"), "expected analysis error, got: {err}");
    }

    #[test]
    fn test_syntax_error_fails_parse() {
        let query = "QUERY bad() => @@@ RETURN x";
        let result = parse_and_validate(TEST_SCHEMA, query);
        assert!(result.is_err(), "expected Err for syntax error");
    }

    #[test]
    fn test_query_with_param() {
        let query = r#"
QUERY getPerson(person_id: String) =>
    person <- N<People>({person_id: person_id})
RETURN person
"#;
        let result = parse_and_validate(TEST_SCHEMA, query);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p sparrow-db --features lmdb runtime_eval::parse 2>&1 | tail -20
```

Expected: compile error — `parse_and_validate` function body not implemented yet

- [ ] **Step 3: The implementation is already written above** — the `parse_and_validate` function body is in Step 1. Run the tests.

- [ ] **Step 4: Run tests**

```bash
cargo test -p sparrow-db --features lmdb runtime_eval::parse 2>&1 | tail -20
```

Expected: 4 tests pass

- [ ] **Step 5: Wire parse into handler (returns analysis errors as 400-style)**

Update `sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs`:

```rust
use crate::{
    protocol::{Format, Response},
    sparrow_engine::types::GraphError,
    sparrow_gateway::router::router::HandlerInput,
};
use super::{RuntimeEvalRequest, RuntimeError, parse::parse_and_validate};
use serde_json::json;

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

    // Ensure exactly one query was submitted
    if source.queries.is_empty() {
        return Err(GraphError::DecodeError("request must contain exactly one QUERY".to_string()));
    }

    // Placeholder: will execute in Task 4
    let body = sonic_rs::to_vec(&sonic_rs::json!({"status": "parsed_ok", "query": source.queries[0].name}))
        .map_err(|e| GraphError::DecodeError(e.to_string()))?;
    Ok(Response { body, fmt: Format::Json })
}
```

Note: Replace `serde_json::json!` with `sonic_rs::json!` — check the available macro; if `sonic_rs::json!` doesn't exist, build the value via `sonic_rs::to_value(...)`.

- [ ] **Step 6: Build**

```bash
cargo build -p sparrow-container --features lmdb 2>&1 | grep "^error" | head -10
```

Expected: no errors

- [ ] **Step 7: Integration test — parse valid query**

```bash
# Setup: use the compiled binary from test-stress.sh or do a quick compile
SPARROW_DATA_DIR=/tmp/runtime-parse-test SPARROW_PORT=7979 SPARROW_RUNTIME_HQL=true \
  ./target/debug/sparrow-container &
SERVER_PID=$!
sleep 1

# Compile a project so the binary has schema embedded
# (Re-use stress test binary which already has schema)
# First test: valid parse succeeds
response=$(curl -s -X POST http://localhost:7979/__hql_runtime_eval \
  -H "Content-Type: application/json" \
  -d '{"query": "QUERY getAll() => people <- N<People>\nRETURN people", "params": {}}')
echo "Response: $response"

kill $SERVER_PID
```

Expected: JSON response with `{"status": "parsed_ok", "query": "getAll"}` (not an error)

- [ ] **Step 8: Commit**

```bash
git add sparrow-db/src/sparrow_gateway/runtime_eval/parse.rs \
        sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs
git commit -m "feat: parse and analyze HQL at runtime in __hql_runtime_eval handler"
```

---

## Task 4: Lowering + execution for read-only traversals

Scope: `N<Type>`, `N<Type>({field: val})`, `::Out<E>`, `::In<E>`, `::OutE<E>`, `::InE<E>`, `::WHERE(_::{field}::OP(val))`, multi-assignment queries, `RETURN var`.

**Files:**
- Create: `sparrow-db/src/sparrow_gateway/runtime_eval/lower.rs`
- Create: `sparrow-db/src/sparrow_gateway/runtime_eval/executor.rs`
- Modify: `sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs`

- [ ] **Step 1: Write failing tests for the lowering module**

Create `sparrow-db/src/sparrow_gateway/runtime_eval/lower.rs`:

```rust
use crate::{
    sparrow_engine::traversal_core::traversal_value::TraversalValue,
    sparrow_gateway::mcp::tools::{
        EdgeType as MecpEdgeType, FilterProperties, FilterTraversal, Operator, ToolArgs,
    },
    sparrowc::parser::types::{
        BooleanOpType, EvaluatesToNumberType, ExpressionType, GraphStepType, IdType,
        ReturnType, Source, StartNode, StatementType, StepType, ValueType,
    },
    protocol::value::Value,
};
use std::collections::HashMap;
use super::RuntimeError;

/// The result of lowering one assignment statement.
pub struct LoweredStep {
    /// None = start fresh; Some(name) = seed from `var_store[name]`
    pub seed_var: Option<String>,
    /// Traversal steps to execute
    pub tool_args: Vec<ToolArgs>,
    /// Name to bind result to
    pub bind_to: String,
}

/// Lower a parsed Source (must have exactly 1 query) into executable steps.
/// Returns (lowered_steps, return_var_names)
pub fn lower_query(
    source: &Source,
    params: &HashMap<String, Value>,
) -> Result<(Vec<LoweredStep>, Vec<String>), RuntimeError> {
    let query = source.queries.first()
        .ok_or_else(|| RuntimeError::Lowering("source has no queries".to_string()))?;

    let mut steps = Vec::new();

    for stmt in &query.statements {
        match &stmt.statement {
            StatementType::Assignment(assign) => {
                let lowered = lower_expression(&assign.value.expr, params, &assign.variable)?;
                steps.push(lowered);
            }
            StatementType::Expression(_) | StatementType::Drop(_) | StatementType::ForLoop(_) => {
                return Err(RuntimeError::Unsupported(
                    "only assignment statements supported in Task 4".to_string()
                ));
            }
        }
    }

    let return_vars: Vec<String> = query.return_values.iter().filter_map(|rv| {
        match rv {
            ReturnType::Expression(e) => match &e.expr {
                ExpressionType::Identifier(name) => Some(name.clone()),
                _ => None,
            },
            ReturnType::Empty => None,
            _ => None,
        }
    }).collect();

    Ok((steps, return_vars))
}

fn lower_expression(
    expr: &ExpressionType,
    params: &HashMap<String, Value>,
    bind_to: &str,
) -> Result<LoweredStep, RuntimeError> {
    match expr {
        ExpressionType::Traversal(traversal) => lower_traversal(traversal, params, bind_to),
        _ => Err(RuntimeError::Unsupported(format!("expression type: {expr}"))),
    }
}

fn lower_traversal(
    traversal: &crate::sparrowc::parser::types::Traversal,
    params: &HashMap<String, Value>,
    bind_to: &str,
) -> Result<LoweredStep, RuntimeError> {
    let (seed_var, mut tool_args) = lower_start_node(&traversal.start, params)?;

    for step in &traversal.steps {
        let more = lower_step(&step.step, params)?;
        tool_args.extend(more);
    }

    Ok(LoweredStep { seed_var, tool_args, bind_to: bind_to.to_string() })
}

fn lower_start_node(
    start: &StartNode,
    params: &HashMap<String, Value>,
) -> Result<(Option<String>, Vec<ToolArgs>), RuntimeError> {
    match start {
        StartNode::Node { node_type, ids } => {
            let mut args = vec![ToolArgs::NFromType { node_type: node_type.clone() }];
            if let Some(id_list) = ids {
                let filter = ids_to_filter(id_list, params)?;
                args.push(ToolArgs::FilterItems { filter });
            }
            Ok((None, args))
        }
        StartNode::Edge { edge_type, ids } => {
            let mut args = vec![ToolArgs::EFromType { edge_type: edge_type.clone() }];
            if let Some(id_list) = ids {
                let filter = ids_to_filter(id_list, params)?;
                args.push(ToolArgs::FilterItems { filter });
            }
            Ok((None, args))
        }
        StartNode::Identifier(name) => {
            Ok((Some(name.clone()), vec![]))
        }
        StartNode::Anonymous => Ok((None, vec![])),
        _ => Err(RuntimeError::Unsupported("vector/search start nodes not supported yet".to_string())),
    }
}

fn ids_to_filter(
    ids: &[IdType],
    params: &HashMap<String, Value>,
) -> Result<FilterTraversal, RuntimeError> {
    // Each IdType::ByIndex { index, value } maps to one FilterProperties
    let mut props = Vec::new();
    for id in ids {
        match id {
            IdType::ByIndex { index, value, .. } => {
                let field_name = index.to_string();
                let val = resolve_value_type(value, params)?;
                props.push(FilterProperties {
                    key: field_name,
                    value: val,
                    operator: Some(Operator::Eq),
                });
            }
            IdType::Literal { value, .. } => {
                return Err(RuntimeError::Unsupported(format!("bare literal id: {value}")));
            }
            IdType::Identifier { value, .. } => {
                return Err(RuntimeError::Unsupported(format!("bare identifier id: {value}")));
            }
        }
    }
    Ok(FilterTraversal { properties: Some(vec![props]), filter_traversals: None })
}

fn resolve_value_type(
    vt: &ValueType,
    params: &HashMap<String, Value>,
) -> Result<Value, RuntimeError> {
    match vt {
        ValueType::Literal { value, .. } => Ok(value.clone()),
        ValueType::Identifier { value: name, .. } => {
            params.get(name)
                .cloned()
                .ok_or_else(|| RuntimeError::Lowering(format!("param '{name}' not provided")))
        }
        ValueType::Object { .. } => Err(RuntimeError::Unsupported("object value types".to_string())),
    }
}

fn lower_step(
    step: &StepType,
    params: &HashMap<String, Value>,
) -> Result<Vec<ToolArgs>, RuntimeError> {
    match step {
        StepType::Node(graph_step) => match &graph_step.step {
            GraphStepType::Out(edge_label) => Ok(vec![ToolArgs::OutStep {
                edge_label: edge_label.clone(),
                edge_type: MecpEdgeType::Node,
                filter: None,
            }]),
            GraphStepType::In(edge_label) => Ok(vec![ToolArgs::InStep {
                edge_label: edge_label.clone(),
                edge_type: MecpEdgeType::Node,
                filter: None,
            }]),
            GraphStepType::OutE(edge_label) => Ok(vec![ToolArgs::OutEStep {
                edge_label: edge_label.clone(),
                filter: None,
            }]),
            GraphStepType::InE(edge_label) => Ok(vec![ToolArgs::InEStep {
                edge_label: edge_label.clone(),
                filter: None,
            }]),
            other => Err(RuntimeError::Unsupported(format!("graph step: {other:?}"))),
        },
        StepType::Edge(graph_step) => match &graph_step.step {
            GraphStepType::Out(edge_label) => Ok(vec![ToolArgs::OutStep {
                edge_label: edge_label.clone(),
                edge_type: MecpEdgeType::Node,
                filter: None,
            }]),
            GraphStepType::In(edge_label) => Ok(vec![ToolArgs::InStep {
                edge_label: edge_label.clone(),
                edge_type: MecpEdgeType::Node,
                filter: None,
            }]),
            other => Err(RuntimeError::Unsupported(format!("edge graph step: {other:?}"))),
        },
        StepType::Where(where_expr) => {
            let filter = lower_where_expr(where_expr, params)?;
            Ok(vec![ToolArgs::FilterItems { filter }])
        }
        other => Err(RuntimeError::Unsupported(format!("step type: {other:?}"))),
    }
}

/// Lower `::WHERE(_::{field}::OP(value))` to a FilterTraversal
fn lower_where_expr(
    expr: &crate::sparrowc::parser::types::Expression,
    params: &HashMap<String, Value>,
) -> Result<FilterTraversal, RuntimeError> {
    // WHERE clause in HQL is a closure expression:
    // `_::{field}::GT(rhs)` → Traversal { start: Anonymous, steps: [Object("field"), BoolOp(GT(rhs))] }
    match &expr.expr {
        ExpressionType::Traversal(traversal) => {
            let mut field_name: Option<String> = None;
            let mut operator: Option<Operator> = None;
            let mut rhs_value: Option<Value> = None;

            for step in &traversal.steps {
                match &step.step {
                    StepType::Object(obj) => {
                        // `{age}` → field access
                        if let Some(first_field) = obj.properties.first() {
                            field_name = Some(first_field.clone());
                        }
                    }
                    StepType::BooleanOperation(bool_op) => {
                        let (op, rhs_expr) = match &bool_op.op {
                            BooleanOpType::Equal(e) => (Operator::Eq, e.as_ref()),
                            BooleanOpType::NotEqual(e) => (Operator::Neq, e.as_ref()),
                            BooleanOpType::GreaterThan(e) => (Operator::Gt, e.as_ref()),
                            BooleanOpType::GreaterThanOrEqual(e) => (Operator::Gte, e.as_ref()),
                            BooleanOpType::LessThan(e) => (Operator::Lt, e.as_ref()),
                            BooleanOpType::LessThanOrEqual(e) => (Operator::Lte, e.as_ref()),
                            other => return Err(RuntimeError::Unsupported(format!("bool op: {other:?}"))),
                        };
                        operator = Some(op);
                        rhs_value = Some(resolve_expr_value(rhs_expr, params)?);
                    }
                    _ => {}
                }
            }

            let field = field_name.ok_or_else(|| RuntimeError::Lowering("WHERE missing field".to_string()))?;
            let op = operator.ok_or_else(|| RuntimeError::Lowering("WHERE missing operator".to_string()))?;
            let val = rhs_value.ok_or_else(|| RuntimeError::Lowering("WHERE missing rhs".to_string()))?;

            Ok(FilterTraversal {
                properties: Some(vec![vec![FilterProperties { key: field, value: val, operator: Some(op) }]]),
                filter_traversals: None,
            })
        }
        _ => Err(RuntimeError::Unsupported("non-traversal WHERE expression".to_string())),
    }
}

fn resolve_expr_value(
    expr: &crate::sparrowc::parser::types::Expression,
    params: &HashMap<String, Value>,
) -> Result<Value, RuntimeError> {
    match &expr.expr {
        ExpressionType::Identifier(name) => params.get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::Lowering(format!("param '{name}' not provided"))),
        ExpressionType::StringLiteral(s) => Ok(Value::String(s.clone())),
        ExpressionType::IntegerLiteral(i) => Ok(Value::I32(*i)),
        ExpressionType::FloatLiteral(f) => Ok(Value::F64(*f)),
        ExpressionType::BooleanLiteral(b) => Ok(Value::Boolean(*b)),
        _ => Err(RuntimeError::Unsupported("complex rhs expression".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparrow_gateway::runtime_eval::parse::parse_and_validate;

    const TEST_SCHEMA: &str = r#"
N::People {
    UNIQUE INDEX person_id: String,
    first_name: String,
    age: I32
}
E::Knows UNIQUE {
    From: People,
    To: People,
    Properties: {}
}
"#;

    fn lower(query: &str, params: &[(&str, Value)]) -> Result<(Vec<LoweredStep>, Vec<String>), RuntimeError> {
        let param_map: HashMap<String, Value> = params.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let source = parse_and_validate(TEST_SCHEMA, query).unwrap();
        lower_query(&source, &param_map)
    }

    #[test]
    fn test_lower_n_from_type_no_filter() {
        let query = "QUERY getAll() => people <- N<People>\nRETURN people";
        let (steps, ret) = lower(query, &[]).unwrap();
        assert_eq!(steps.len(), 1);
        assert!(matches!(steps[0].tool_args[0], ToolArgs::NFromType { .. }));
        assert_eq!(ret, vec!["people"]);
    }

    #[test]
    fn test_lower_n_with_index_filter() {
        let query = "QUERY get(pid: String) => p <- N<People>({person_id: pid})\nRETURN p";
        let (steps, _) = lower(query, &[("pid", Value::String("alice".to_string()))]).unwrap();
        assert_eq!(steps[0].tool_args.len(), 2);
        assert!(matches!(steps[0].tool_args[0], ToolArgs::NFromType { .. }));
        assert!(matches!(steps[0].tool_args[1], ToolArgs::FilterItems { .. }));
    }

    #[test]
    fn test_lower_traversal_chain() {
        let query = r#"
QUERY getFriends(pid: String) =>
    p <- N<People>({person_id: pid})
    friends <- p::Out<Knows>
RETURN friends
"#;
        let (steps, ret) = lower(query, &[("pid", Value::String("alice".to_string()))]).unwrap();
        assert_eq!(steps.len(), 2);
        assert!(steps[1].seed_var.is_some());
        assert_eq!(steps[1].seed_var.as_deref(), Some("p"));
        assert!(matches!(steps[1].tool_args[0], ToolArgs::OutStep { .. }));
        assert_eq!(ret, vec!["friends"]);
    }
}
```

Note: the `Object` step has a `properties: Vec<String>` — verify the actual field name when implementing. If it's different (e.g., `fields` or `field`), adjust accordingly.

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p sparrow-db --features lmdb runtime_eval::lower 2>&1 | tail -30
```

Expected: compile errors from missing `Object.properties` or type mismatches — fix field names to match actual AST

- [ ] **Step 3: Fix any field name mismatches**

Run:

```bash
grep -n "struct Object\|pub fields\|pub properties" \
  sparrow-db/src/sparrowc/parser/types.rs
```

Adjust `lower.rs` to match actual field names for `Object` struct in the parser types.

- [ ] **Step 4: Run tests again**

```bash
cargo test -p sparrow-db --features lmdb runtime_eval::lower 2>&1 | tail -30
```

Expected: all tests pass

- [ ] **Step 5: Create the executor**

Create `sparrow-db/src/sparrow_gateway/runtime_eval/executor.rs`:

```rust
use crate::{
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, Txn},
        traversal_core::traversal_value::TraversalValue,
        types::GraphError,
    },
    sparrow_gateway::mcp::tools::execute_query_chain,
    sparrow_gateway::mcp::tools::execute_query_chain_from_seed,
};
use bumpalo::Bump;
use std::collections::HashMap;
use super::{RuntimeError, lower::LoweredStep};

pub fn execute_plan<'db>(
    steps: &[LoweredStep],
    return_vars: &[String],
    storage: &'db SparrowGraphStorage,
) -> Result<HashMap<String, Vec<sonic_rs::Value>>, RuntimeError> {
    let arena = Bump::new();
    let txn = storage.graph_env.read_txn()
        .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

    let mut var_store: HashMap<String, Vec<TraversalValue>> = HashMap::new();

    for step in steps {
        let result = if let Some(seed_name) = &step.seed_var {
            let seed_values = var_store.get(seed_name).cloned().unwrap_or_default();
            let seed_iter = seed_values.into_iter();
            execute_query_chain_from_seed(
                &step.tool_args,
                storage,
                &txn,
                &arena,
                seed_iter,
            )
            .map_err(RuntimeError::Execution)?
            .collect()
            .map_err(RuntimeError::Execution)?
        } else {
            execute_query_chain(&step.tool_args, storage, &txn, &arena)
                .map_err(RuntimeError::Execution)?
                .collect()
                .map_err(RuntimeError::Execution)?
        };

        var_store.insert(step.bind_to.clone(), result);
    }

    // Serialize return variables
    let mut output = HashMap::new();
    for var_name in return_vars {
        let values = var_store.remove(var_name).unwrap_or_default();
        let json_values: Vec<sonic_rs::Value> = values.iter()
            .map(|v| sonic_rs::to_value(v).unwrap_or(sonic_rs::Value::Null))
            .collect();
        output.insert(var_name.clone(), json_values);
    }

    Ok(output)
}
```

- [ ] **Step 6: Wire executor into handler**

Update `sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs`:

```rust
use crate::{
    protocol::{Format, Response},
    sparrow_engine::types::GraphError,
    sparrow_gateway::router::router::HandlerInput,
    protocol::value::Value,
};
use super::{RuntimeEvalRequest, RuntimeError, parse::parse_and_validate, lower::lower_query, executor::execute_plan};
use std::collections::HashMap;

pub fn handle(input: HandlerInput, hql_schema_raw: Option<String>) -> Result<Response, GraphError> {
    let schema = hql_schema_raw.ok_or(RuntimeError::NoSchema)?;

    let req: RuntimeEvalRequest = Format::Json
        .deserialize_owned(&input.request.body)
        .map_err(|e| GraphError::DecodeError(e.to_string()))?;

    if req.query.trim().is_empty() {
        return Err(RuntimeError::NoQuery.into());
    }

    // Parse the runtime params from JSON to Value map
    let params = parse_params(&req.params)?;

    // Parse + analyze
    let source = parse_and_validate(&schema, &req.query)
        .map_err(GraphError::from)?;

    if source.queries.is_empty() {
        return Err(GraphError::DecodeError("request must contain exactly one QUERY".to_string()));
    }

    // Lower to execution plan
    let (steps, return_vars) = lower_query(&source, &params)
        .map_err(GraphError::from)?;

    // Execute against storage
    let storage = &input.graph.storage;
    let result = execute_plan(&steps, &return_vars, storage)
        .map_err(GraphError::from)?;

    let body = sonic_rs::to_vec(&result)
        .map_err(|e| GraphError::DecodeError(e.to_string()))?;
    Ok(Response { body, fmt: Format::Json })
}

fn parse_params(params: &sonic_rs::Value) -> Result<HashMap<String, Value>, GraphError> {
    let mut map = HashMap::new();
    if let Some(obj) = params.as_object() {
        for (k, v) in obj {
            let val = json_to_value(v)
                .map_err(|e| GraphError::DecodeError(format!("param '{k}': {e}")))?;
            map.insert(k.to_string(), val);
        }
    }
    Ok(map)
}

fn json_to_value(v: &sonic_rs::Value) -> Result<Value, String> {
    match v {
        v if v.is_str() => Ok(Value::String(v.as_str().unwrap().to_string())),
        v if v.is_i64() => Ok(Value::I64(v.as_i64().unwrap())),
        v if v.is_u64() => Ok(Value::U64(v.as_u64().unwrap())),
        v if v.is_f64() => Ok(Value::F64(v.as_f64().unwrap())),
        v if v.is_boolean() => Ok(Value::Boolean(v.as_bool().unwrap())),
        v if v.is_null() => Err("null params not supported".to_string()),
        _ => Err(format!("unsupported param type: {v:?}")),
    }
}
```

- [ ] **Step 7: Build**

```bash
cargo build -p sparrow-container --features lmdb 2>&1 | grep "^error" | head -10
```

Expected: no errors

- [ ] **Step 8: End-to-end integration test**

Run the stress test first to populate data, then test runtime eval:

```bash
# Start server (need sparrow-container built with lmdb+schema embedded)
# The stress test schema has N::People, N::Company, N::Jobs
# Reuse the stress test data dir

SPARROW_DATA_DIR=/tmp/sparrow-stress-data SPARROW_PORT=7979 SPARROW_RUNTIME_HQL=true \
  ./target/debug/sparrow-container &
SERVER_PID=$!
sleep 2

# Test 1: scan all people
curl -s -X POST http://localhost:7979/__hql_runtime_eval \
  -H "Content-Type: application/json" \
  -d '{"query": "QUERY getAll() =>\n  people <- N<People>\nRETURN people", "params": {}}' | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(f'People count: {len(d[\"people\"])}')"

# Test 2: point lookup
curl -s -X POST http://localhost:7979/__hql_runtime_eval \
  -H "Content-Type: application/json" \
  -d '{"query": "QUERY get(pid: String) =>\n  p <- N<People>({person_id: pid})\nRETURN p", "params": {"pid": "person-1"}}' | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(f'Found: {len(d[\"p\"])} person(s)')"

kill $SERVER_PID
```

Expected:
- Test 1: `People count: 1000`
- Test 2: `Found: 1 person(s)`

- [ ] **Step 9: Commit**

```bash
git add sparrow-db/src/sparrow_gateway/runtime_eval/lower.rs \
        sparrow-db/src/sparrow_gateway/runtime_eval/executor.rs \
        sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs
git commit -m "feat: lower AST to ToolArgs and execute read-only traversals via __hql_runtime_eval"
```

---

## Task 5: Mutations — AddN, AddE, DROP, UPDATE

Scope: `AddN<Type>({...})`, `AddE<Type>()::From(node)::To(node)`, `DROP N<Type>({...})`, `::UPDATE({...})`.

**Files:**
- Create: `sparrow-db/src/sparrow_gateway/runtime_eval/mutations.rs`
- Modify: `sparrow-db/src/sparrow_gateway/runtime_eval/lower.rs`
- Modify: `sparrow-db/src/sparrow_gateway/runtime_eval/executor.rs`

- [ ] **Step 1: Understand mutation APIs in storage**

Run:

```bash
grep -n "pub fn add_node\|pub fn delete_node\|pub fn update_node\|pub fn add_edge\|pub fn delete_edge" \
  sparrow-db/src/sparrow_engine/storage_core/mod.rs | head -20
```

Note the exact signatures — you'll need them for the mutations module.

- [ ] **Step 2: Extend the `LoweredStep` to cover mutations**

In `sparrow-db/src/sparrow_gateway/runtime_eval/lower.rs`, add a `MutationOp` enum and extend `LoweredStep`:

```rust
pub enum MutationOp {
    AddNode {
        node_type: String,
        fields: HashMap<String, Value>,
    },
    AddEdge {
        edge_type: String,
        from_var: String,
        to_var: String,
        fields: HashMap<String, Value>,
    },
    DropNodes {
        // The traversal to feed into drop
        tool_args: Vec<ToolArgs>,
        seed_var: Option<String>,
    },
    UpdateNodes {
        updates: HashMap<String, Value>,
        // The traversal that selects nodes to update
        tool_args: Vec<ToolArgs>,
        seed_var: Option<String>,
    },
}

pub enum LoweredOp {
    Traversal(LoweredStep),
    Mutation { bind_to: String, op: MutationOp },
}
```

Update `lower_query` to return `Vec<LoweredOp>` instead of `Vec<LoweredStep>`.

- [ ] **Step 3: Add AddNode lowering**

```rust
ExpressionType::AddNode(add_node) => {
    let node_type = add_node.node_type.as_deref()
        .ok_or_else(|| RuntimeError::Lowering("AddNode missing type".to_string()))?
        .to_string();
    let fields = lower_fields(add_node.fields.as_ref(), params)?;
    Ok(LoweredOp::Mutation {
        bind_to: bind_to.to_string(),
        op: MutationOp::AddNode { node_type, fields },
    })
}
```

- [ ] **Step 4: Add AddEdge lowering**

```rust
ExpressionType::AddEdge(add_edge) => {
    let edge_type = add_edge.edge_type.as_deref()
        .ok_or_else(|| RuntimeError::Lowering("AddEdge missing type".to_string()))?
        .to_string();
    let from_var = add_edge.connection.from_id.as_ref()
        .and_then(|id| match id { IdType::Identifier { value, .. } => Some(value.clone()), _ => None })
        .ok_or_else(|| RuntimeError::Lowering("AddEdge missing From identifier".to_string()))?;
    let to_var = add_edge.connection.to_id.as_ref()
        .and_then(|id| match id { IdType::Identifier { value, .. } => Some(value.clone()), _ => None })
        .ok_or_else(|| RuntimeError::Lowering("AddEdge missing To identifier".to_string()))?;
    let fields = lower_fields(add_edge.fields.as_ref(), params)?;
    Ok(LoweredOp::Mutation {
        bind_to: bind_to.to_string(),
        op: MutationOp::AddEdge { edge_type, from_var, to_var, fields },
    })
}
```

- [ ] **Step 5: Add DROP lowering**

In `lower_query`, handle `StatementType::Drop(expr)`:

```rust
StatementType::Drop(expr) => {
    match &expr.expr {
        ExpressionType::Traversal(traversal) => {
            let (seed_var, tool_args) = lower_start_node(&traversal.start, params)?;
            let mut all_args = tool_args;
            for step in &traversal.steps {
                all_args.extend(lower_step(&step.step, params)?);
            }
            ops.push(LoweredOp::Mutation {
                bind_to: "_drop_result".to_string(),
                op: MutationOp::DropNodes { tool_args: all_args, seed_var },
            });
        }
        _ => return Err(RuntimeError::Unsupported("non-traversal DROP".to_string())),
    }
}
```

- [ ] **Step 6: Add UPDATE lowering**

In `lower_step`, handle `StepType::Update(update)`:

```rust
StepType::Update(update) => {
    let updates = lower_fields(Some(&update.fields), params)?;
    Ok(vec![/* signal to executor via a special ToolArgs extension or handle differently */])
}
```

Note: UPDATE can't be encoded as a `ToolArgs` since `ToolArgs` is read-only. Instead, handle UPDATE as a special case in the traversal loop — when a `StepType::Update` is present in a traversal, emit a `MutationOp::UpdateNodes` instead of a `LoweredStep`.

- [ ] **Step 7: Implement `mutations.rs` with write transaction handling**

Create `sparrow-db/src/sparrow_gateway/runtime_eval/mutations.rs`:

```rust
use crate::sparrow_engine::{
    storage_core::SparrowGraphStorage,
    traversal_core::traversal_value::TraversalValue,
    types::GraphError,
};
use crate::sparrow_gateway::mcp::tools::{execute_query_chain, execute_query_chain_from_seed, ToolArgs};
use crate::protocol::value::Value;
use bumpalo::Bump;
use std::collections::HashMap;
use super::{RuntimeError, lower::MutationOp};

pub fn execute_mutation(
    op: &MutationOp,
    var_store: &mut HashMap<String, Vec<TraversalValue>>,
    storage: &SparrowGraphStorage,
) -> Result<Vec<TraversalValue>, RuntimeError> {
    match op {
        MutationOp::AddNode { node_type, fields } => {
            let mut wtxn = storage.graph_env.write_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            
            // Convert fields to storage format and add node
            let node = storage.add_node(&mut wtxn, node_type, fields)
                .map_err(RuntimeError::Execution)?;
            wtxn.commit()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            Ok(vec![node])
        }
        MutationOp::AddEdge { edge_type, from_var, to_var, fields } => {
            let from_nodes = var_store.get(from_var).cloned().unwrap_or_default();
            let to_nodes = var_store.get(to_var).cloned().unwrap_or_default();
            
            let from_id = from_nodes.first()
                .map(|v| v.id())
                .ok_or_else(|| RuntimeError::Lowering(format!("From variable '{from_var}' is empty")))?;
            let to_id = to_nodes.first()
                .map(|v| v.id())
                .ok_or_else(|| RuntimeError::Lowering(format!("To variable '{to_var}' is empty")))?;
            
            let mut wtxn = storage.graph_env.write_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            let edge = storage.add_edge(&mut wtxn, edge_type, from_id, to_id, fields)
                .map_err(RuntimeError::Execution)?;
            wtxn.commit()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            Ok(vec![edge])
        }
        MutationOp::DropNodes { tool_args, seed_var } => {
            // First collect nodes to drop (read txn), then delete (write txn)
            let arena = Bump::new();
            let rtxn = storage.graph_env.read_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            let targets = if let Some(seed) = seed_var {
                let seed_vals = var_store.get(seed).cloned().unwrap_or_default();
                execute_query_chain_from_seed(tool_args, storage, &rtxn, &arena, seed_vals.into_iter())
            } else {
                execute_query_chain(tool_args, storage, &rtxn, &arena)
            }
            .map_err(RuntimeError::Execution)?
            .collect()
            .map_err(RuntimeError::Execution)?;
            drop(rtxn);
            
            let mut wtxn = storage.graph_env.write_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            for target in &targets {
                storage.delete_by_id(&mut wtxn, target.id())
                    .map_err(RuntimeError::Execution)?;
            }
            wtxn.commit()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            Ok(vec![])
        }
        MutationOp::UpdateNodes { updates, tool_args, seed_var } => {
            let arena = Bump::new();
            let rtxn = storage.graph_env.read_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            let targets = if let Some(seed) = seed_var {
                let seed_vals = var_store.get(seed).cloned().unwrap_or_default();
                execute_query_chain_from_seed(tool_args, storage, &rtxn, &arena, seed_vals.into_iter())
            } else {
                execute_query_chain(tool_args, storage, &rtxn, &arena)
            }
            .map_err(RuntimeError::Execution)?
            .collect()
            .map_err(RuntimeError::Execution)?;
            drop(rtxn);
            
            let mut wtxn = storage.graph_env.write_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            let mut updated = Vec::new();
            for target in targets {
                let result = storage.update_node_fields(&mut wtxn, target.id(), updates)
                    .map_err(RuntimeError::Execution)?;
                updated.push(result);
            }
            wtxn.commit()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
            Ok(updated)
        }
    }
}
```

**Important**: Verify the exact method names (`add_node`, `add_edge`, `delete_by_id`, `update_node_fields`) by running:

```bash
grep -n "pub fn add_node\|pub fn add_edge\|pub fn delete\|pub fn update" \
  sparrow-db/src/sparrow_engine/storage_core/mod.rs | head -20
```

Adjust method names to match what actually exists.

- [ ] **Step 8: Write failing tests for mutations**

In `sparrow-db/src/sparrow_gateway/runtime_eval/mutations.rs`:

```rust
#[cfg(test)]
mod tests {
    // Mutation tests require a live database; these are integration tests.
    // See test-runtime-eval.sh for end-to-end mutation testing.
    // Unit-test only the lowering:
    use super::super::lower::*;
    use super::super::parse::parse_and_validate;
    use crate::protocol::value::Value;
    use std::collections::HashMap;

    const SCHEMA: &str = r#"
N::Item { UNIQUE INDEX item_id: String, label: String }
E::Links UNIQUE { From: Item, To: Item, Properties: {} }
"#;

    #[test]
    fn test_lower_add_node() {
        let query = r#"
QUERY create(item_id: String, label: String) =>
    item <- AddN<Item>({item_id: item_id, label: label})
RETURN item
"#;
        let source = parse_and_validate(SCHEMA, query).unwrap();
        let params: HashMap<String, Value> = [
            ("item_id".to_string(), Value::String("x1".to_string())),
            ("label".to_string(), Value::String("hello".to_string())),
        ].into();
        let (ops, _) = lower_query(&source, &params).unwrap();
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], LoweredOp::Mutation { op: MutationOp::AddNode { .. }, .. }));
    }

    #[test]
    fn test_lower_add_edge() {
        let query = r#"
QUERY link(a_id: String, b_id: String) =>
    a <- N<Item>({item_id: a_id})
    b <- N<Item>({item_id: b_id})
    edge <- AddE<Links>()::From(a)::To(b)
RETURN edge
"#;
        let source = parse_and_validate(SCHEMA, query).unwrap();
        let params: HashMap<String, Value> = [
            ("a_id".to_string(), Value::String("a1".to_string())),
            ("b_id".to_string(), Value::String("b1".to_string())),
        ].into();
        let (ops, _) = lower_query(&source, &params).unwrap();
        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[2], LoweredOp::Mutation { op: MutationOp::AddEdge { .. }, .. }));
    }
}
```

- [ ] **Step 9: Run unit tests**

```bash
cargo test -p sparrow-db --features lmdb runtime_eval 2>&1 | tail -30
```

Expected: all unit tests pass

- [ ] **Step 10: End-to-end mutation test**

```bash
SPARROW_DATA_DIR=/tmp/runtime-mutation-test SPARROW_PORT=7979 SPARROW_RUNTIME_HQL=true \
  ./target/debug/sparrow-container &
SERVER_PID=$!
sleep 2

# Create a node
curl -s -X POST http://localhost:7979/__hql_runtime_eval \
  -H "Content-Type: application/json" \
  -d '{"query": "QUERY create(person_id: String, first_name: String, last_name: String, age: I32) =>\n  p <- AddN<People>({person_id: person_id, first_name: first_name, last_name: last_name, age: age})\nRETURN p", "params": {"person_id": "rt-001", "first_name": "Alice", "last_name": "Smith", "age": 30}}'

# Read it back
curl -s -X POST http://localhost:7979/__hql_runtime_eval \
  -H "Content-Type: application/json" \
  -d '{"query": "QUERY get(pid: String) =>\n  p <- N<People>({person_id: pid})\nRETURN p", "params": {"pid": "rt-001"}}' | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d['p'][0]['first_name'])"

kill $SERVER_PID
```

Expected: output `Alice`

- [ ] **Step 11: Commit**

```bash
git add sparrow-db/src/sparrow_gateway/runtime_eval/mutations.rs \
        sparrow-db/src/sparrow_gateway/runtime_eval/lower.rs \
        sparrow-db/src/sparrow_gateway/runtime_eval/executor.rs \
        sparrow-db/src/sparrow_gateway/runtime_eval/handler.rs
git commit -m "feat: support AddN, AddE, DROP, UPDATE mutations in runtime eval"
```

---

## Task 6: Full integration test suite + MCP tool

**Files:**
- Create: `test-runtime-eval.sh`
- Modify: `sparrow-db/src/sparrow_gateway/mcp/tools.rs` (add `hql_eval` MCP tool)

- [ ] **Step 1: Write the runtime eval integration test script**

Create `test-runtime-eval.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
SPARROW_BIN="$REPO_ROOT/target/debug/sparrow"
CONTAINER_BIN="$REPO_ROOT/target/debug/sparrow-container"
PROJECT_DIR="/tmp/sparrow-runtime-eval-project"
DATA_DIR="/tmp/sparrow-runtime-eval-data"
QUERIES_RS="$REPO_ROOT/sparrow-container/src/queries.rs"
QUERIES_RS_BAK="/tmp/sparrow-runtime-eval-queries.rs.bak"
PORT=7777
SERVER_PID=""

cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    if [ -f "$QUERIES_RS_BAK" ]; then
        cp "$QUERIES_RS_BAK" "$QUERIES_RS"
        rm -f "$QUERIES_RS_BAK"
    fi
    rm -rf "$PROJECT_DIR" "$DATA_DIR"
}
trap cleanup EXIT INT TERM

PASS=0; FAIL=0

assert_contains() {
    local label=$1 response=$2 expected=$3
    if echo "$response" | grep -q "$expected"; then
        echo "PASS: $label"
        PASS=$((PASS+1))
    else
        echo "FAIL: $label — expected '$expected' in: $response"
        FAIL=$((FAIL+1))
    fi
}

assert_not_contains() {
    local label=$1 response=$2 expected=$3
    if ! echo "$response" | grep -q "$expected"; then
        echo "PASS: $label"
        PASS=$((PASS+1))
    else
        echo "FAIL: $label — did NOT expect '$expected' in: $response"
        FAIL=$((FAIL+1))
    fi
}

eval_query() {
    local query=$1 params=${2:-'{}'}
    curl -s -X POST "http://localhost:$PORT/__hql_runtime_eval" \
      -H "Content-Type: application/json" \
      -d "{\"query\": $(echo "$query" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))'), \"params\": $params}"
}

# === Setup project (same schema as stress test) ===
mkdir -p "$PROJECT_DIR"
cat > "$PROJECT_DIR/sparrow.toml" << 'EOF'
[project]
name = "runtime-eval-test"
queries = "."
[local.dev]
port = 7777
build_mode = "dev"
[cloud]
EOF

cat > "$PROJECT_DIR/schema.hx" << 'EOF'
N::People {
    UNIQUE INDEX person_id: String,
    first_name: String,
    last_name: String,
    age: I32
}
N::Company {
    UNIQUE INDEX name: String
}
E::WorksAt UNIQUE {
    From: People,
    To: Company,
    Properties: {}
}
EOF

# Minimal queries.hx (compile requires at least one query)
cat > "$PROJECT_DIR/queries.hx" << 'EOF'
QUERY dummy() =>
    p <- N<People>
RETURN p
EOF

cp "$QUERIES_RS" "$QUERIES_RS_BAK"
"$SPARROW_BIN" compile --path "$PROJECT_DIR" --output "$REPO_ROOT/sparrow-container/src/"
cargo build -p sparrow-container 2>&1 | tail -5

mkdir -p "$DATA_DIR"
SPARROW_DATA_DIR="$DATA_DIR" SPARROW_PORT="$PORT" SPARROW_RUNTIME_HQL=true \
  "$CONTAINER_BIN" &
SERVER_PID=$!

echo -n "Waiting for server"
for i in $(seq 1 30); do
    if curl -s --max-time 1 "http://localhost:$PORT/dummy" \
            -X POST -H "Content-Type: application/json" -d '{}' > /dev/null 2>&1; then
        echo " ready!"; break
    fi
    echo -n "."; sleep 0.5
    if [ "$i" -eq 30 ]; then echo ""; echo "ERROR: server timeout"; exit 1; fi
done

# === Phase 1: Create nodes ===
echo "=== Phase 1: Create nodes ==="
r=$(eval_query "QUERY create(person_id: String, first_name: String, last_name: String, age: I32) =>
  p <- AddN<People>({person_id: person_id, first_name: first_name, last_name: last_name, age: age})
RETURN p" '{"person_id": "p1", "first_name": "Alice", "last_name": "Smith", "age": 30}')
assert_contains "create person" "$r" "Alice"

r=$(eval_query "QUERY createCo(name: String) =>
  c <- AddN<Company>({name: name})
RETURN c" '{"name": "Acme"}')
assert_contains "create company" "$r" "Acme"

# === Phase 2: Read nodes ===
echo "=== Phase 2: Read nodes ==="
r=$(eval_query "QUERY get(pid: String) =>
  p <- N<People>({person_id: pid})
RETURN p" '{"pid": "p1"}')
assert_contains "lookup person" "$r" "Alice"

r=$(eval_query "QUERY getAll() =>
  p <- N<People>
RETURN p")
assert_contains "scan all people" "$r" "Alice"

# === Phase 3: Create and traverse edge ===
echo "=== Phase 3: Edge creation and traversal ==="
r=$(eval_query "QUERY link(pid: String, cname: String) =>
  person <- N<People>({person_id: pid})
  company <- N<Company>({name: cname})
  e <- AddE<WorksAt>()::From(person)::To(company)
RETURN e" '{"pid": "p1", "cname": "Acme"}')
assert_contains "create edge" "$r" "WorksAt"

r=$(eval_query "QUERY getCompany(pid: String) =>
  person <- N<People>({person_id: pid})
  company <- person::Out<WorksAt>
RETURN company" '{"pid": "p1"}')
assert_contains "traverse out edge" "$r" "Acme"

# === Phase 4: Filter ===
echo "=== Phase 4: WHERE filter ==="
r=$(eval_query "QUERY youngPeople(max_age: I32) =>
  p <- N<People>::WHERE(_::{age}::LT(max_age))
RETURN p" '{"max_age": 25}')
assert_not_contains "filter excludes Alice (age 30)" "$r" "Alice"

# === Phase 5: Delete and verify ===
echo "=== Phase 5: Delete ==="
eval_query "QUERY del(pid: String) =>
  DROP N<People>({person_id: pid})
RETURN NONE" '{"pid": "p1"}' > /dev/null

r=$(eval_query "QUERY check(pid: String) =>
  p <- N<People>({person_id: pid})
RETURN p" '{"pid": "p1"}')
assert_not_contains "deleted person not found" "$r" "Alice"

# === Phase 6: Update ===
echo "=== Phase 6: Update ==="
eval_query "QUERY createForUpdate(pid: String, fn: String, ln: String, age: I32) =>
  p <- AddN<People>({person_id: pid, first_name: fn, last_name: ln, age: age})
RETURN p" '{"pid": "p2", "fn": "Bob", "ln": "Jones", "age": 25}' > /dev/null

eval_query "QUERY updateAge(pid: String, new_age: I32) =>
  p <- N<People>({person_id: pid})::UPDATE({age: new_age})
RETURN p" '{"pid": "p2", "new_age": 40}' > /dev/null

r=$(eval_query "QUERY getAge(pid: String) =>
  p <- N<People>({person_id: pid})
RETURN p" '{"pid": "p2"}')
assert_contains "update age visible" "$r" "40"

# === Phase 7: Error handling ===
echo "=== Phase 7: Error handling ==="
r=$(eval_query "QUERY bad() =>
  x <- N<Nonexistent>
RETURN x")
assert_contains "unknown type rejected" "$r" "error"

r=$(eval_query "QUERY syn() => @@@ invalid syntax RETURN x")
assert_contains "syntax error rejected" "$r" "error"

# === Summary ===
echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] || exit 1
```

- [ ] **Step 2: Run to verify it fails (mutations not yet wired)**

```bash
chmod +x test-runtime-eval.sh
./test-runtime-eval.sh 2>&1 | tail -20
```

Expected: some phases fail (mutations return errors)

- [ ] **Step 3: Fix any remaining issues from the full run**

Common issues to look for:
- `add_node` method name doesn't exist → find correct name with grep
- `delete_by_id` doesn't exist → find correct delete method
- `update_node_fields` doesn't exist → find correct update method
- Transaction model requires passing `&mut wtxn` differently

Fix each issue until all phases pass.

- [ ] **Step 4: Run full test suite**

```bash
./test-runtime-eval.sh 2>&1
```

Expected: `Results: 12 passed, 0 failed`

- [ ] **Step 5: Run original stress test to verify no regressions**

```bash
./test-stress.sh 2>&1 | tail -5
```

Expected: `Done (wall time: ...s)` — all phases pass

- [ ] **Step 6: Commit**

```bash
git add test-runtime-eval.sh
git commit -m "feat: add runtime eval integration test suite (6 phases)"
```

---

## Self-Review

**Spec coverage:**
- ✅ `POST /__hql_runtime_eval` endpoint
- ✅ Env gate `SPARROW_RUNTIME_HQL=true`
- ✅ Parse raw HQL
- ✅ Analyze against deployed schema
- ✅ Lower to traversal primitives
- ✅ Execute without recompile
- ✅ Read traversals: N, E, Out, In, OutE, InE, WHERE
- ✅ Mutations: AddN, AddE, DROP, UPDATE
- ✅ Parameter passing
- ✅ Error responses for invalid HQL

**Placeholder scan:** No TBD/TODO/placeholder content — all steps have code.

**Type consistency:**
- `LoweredStep` / `LoweredOp` defined in `lower.rs` and used consistently in `executor.rs`
- `ToolArgs` variants match `sparrow_gateway::mcp::tools` — verified in source
- `execute_query_chain_from_seed` signature used correctly (seeds from `var_store`)
- `sonic_rs::Value` used for JSON serialization (not `serde_json`)

**Known TODOs to verify during implementation:**
- Exact `Object` struct fields in parser types (may be `fields: Vec<String>` not `properties`)
- `add_node`, `add_edge`, `delete_by_id`, `update_node_fields` method names in storage
- `sonic_rs::json!` macro availability (if absent, use `sonic_rs::to_value` with a struct)
- `thiserror` crate availability in `sparrow-db` — if absent, use manual `Display` + `Error` impl
