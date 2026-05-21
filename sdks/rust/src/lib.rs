
//! # sparrow-sdk Rust SDK
//!
//! Crate root. The query-builder DSL lives in [`dsl`] and the query bundle /
//! code-generation support lives in [`query_generator`].
//!
//! Most application code only needs the curated builder API:
//! ```
//! use sparrow_sdk::dsl::prelude::*;
//! ```

pub mod dsl;
pub mod query_generator;

use std::marker::PhantomData;

// Re-export the DSL surface (types, builders, `prelude`, etc.) at the crate
// root. This is also what makes the `crate::*` paths used inside `dsl.rs` and
// `query_generator.rs` resolve.
pub use dsl::*;

// Convenience re-export so `sparrow_sdk::prelude::*` is reachable directly, in
// addition to the canonical `sparrow_sdk::dsl::prelude::*`.
pub use dsl::prelude;

use reqwest::{Client as ReqwestClient, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Async HTTP client for running queries against a SparrowDB instance.
///
/// Reachable as `sparrow_sdk::Client`.
#[derive(Debug, Clone)]
pub struct Client {
    client: ReqwestClient,
    url: reqwest::Url,
    api_key: Option<String>,
}

/// Backwards-compatible alias for [`Client`].
pub type SparrowDBClient = Client;

#[derive(Debug, Error)]
pub enum SparrowError {
    #[error("Error communicating with server: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Got Error from server: {details}")]
    RemoteError { details: String },
    #[error("Error serializing data: {0}")]
    SerializationError(#[from] sonic_rs::Error),
    #[error("Invalid URL: {0}")]
    InvalidURL(String),
}

impl Client {
    pub fn new(url: Option<&str>) -> Result<Self, SparrowError> {
        // Resolve the base query endpoint up front. `send()` reuses this for
        // dynamic queries and appends `/<name>` for stored queries.
        let url = reqwest::Url::parse(url.unwrap_or("http://localhost:6969"))
            .map_err(|e| SparrowError::InvalidURL(e.to_string()))?
            .join("/v1/query")
            .map_err(|e| SparrowError::InvalidURL(e.to_string()))?;
        Ok(Self {
            client: ReqwestClient::new(),
            url,
            api_key: None,
        })
    }

    pub fn with_api_key(mut self, api_key: Option<&str>) -> Self {
        self.api_key = api_key.map(|key| key.to_string());
        self
    }

    pub fn query<R: for<'de> Deserialize<'de>>(&self) -> QueryBuilder<'_, '_, R> {
        QueryBuilder::new(self)
    }
}

pub struct QueryBuilder<'hlx, 'a, R> {
    client: &'hlx SparrowDBClient,
    query_type: QueryType,
    headers: [Option<(&'a str, &'a str)>; 4],
    body: Option<Vec<u8>>,
    _phantom: PhantomData<R>,
}

#[derive(Default)]
pub(crate) enum QueryType {
    Stored(String),
    Dynamic(DynamicQueryRequest),
    #[default]
    Empty,
}

impl<'hlx, 'a, R> QueryBuilder<'hlx, 'a, R> {
    pub fn new(client: &'hlx SparrowDBClient) -> Self {
        let mut headers = [None; 4];
        headers[0] = Some(("Content-Type", "application/json"));
        Self {
            client,
            query_type: QueryType::default(),
            headers,
            body: None,
            _phantom: PhantomData,
        }
    }

    pub fn writer_only(mut self) -> Self {
        self.headers[1] = Some(("x-helix-require-writer", "true"));
        self
    }

    #[must_use]
    pub fn warm_only(mut self) -> Self {
        self.headers[2] = Some(("x-helix-warm", "true"));
        self
    }

    pub fn should_await_durability(mut self, should: bool) -> Self {
        self.headers[3] = Some((
            "x-helix-await-durable",
            if should { "true" } else { "false" },
        ));
        self
    }

    pub fn body<T: Serialize + Sync>(mut self, data: &T) -> Result<Self, SparrowError> {
        self.body = Some(sonic_rs::to_vec(data)?);
        Ok(self)
    }

    pub fn stored_query(mut self, query_name: String) -> QueryRequest<'hlx, 'a, R> {
        self.query_type = QueryType::Stored(query_name);
        QueryRequest { request: self }
    }

    pub fn dynamic_query(mut self, query: DynamicQueryRequest) -> QueryRequest<'hlx, 'a, R> {
        self.query_type = QueryType::Dynamic(query);
        QueryRequest { request: self }
    }
}

pub struct QueryRequest<'hlx, 'a, R> {
    request: QueryBuilder<'hlx, 'a, R>,
}

impl<'hlx, 'a, R: for<'de> Deserialize<'de>> QueryRequest<'hlx, 'a, R> {
    pub async fn send(self) -> Result<R, SparrowError> {
        let query_request = self.request;
        let (url, body) = match query_request.query_type {
            QueryType::Dynamic(query) => ("/v1/query".to_string(), Some(sonic_rs::to_vec(&query)?)),
            QueryType::Stored(name) => (format!("/v1/query/{name}"), query_request.body),
            QueryType::Empty => unreachable!(
                "send() is only reachable after stored_query() or dynamic_query() sets query_type"
            ),
        };
        let url = query_request
            .client
            .url
            .join(&url)
            .map_err(|e| SparrowError::InvalidURL(e.to_string()))?;

        let mut request = query_request.client.client.post(url);

        for (k, v) in query_request.headers.into_iter().flatten() {
            request = request.header(k, v);
        }
        if let Some(ref api_key) = query_request.client.api_key {
            request = request.bearer_auth(api_key);
        }
        if let Some(body) = body {
            request = request.body(body);
        }

        let response = request.send().await?;

        match response.status() {
            StatusCode::OK => {
                let bytes = response.bytes().await?;
                sonic_rs::from_slice::<R>(&bytes).map_err(Into::into)
            }
            code => match response.text().await {
                Ok(t) => Err(SparrowError::RemoteError { details: t }),
                Err(_) => match code.canonical_reason() {
                    Some(r) => Err(SparrowError::RemoteError {
                        details: r.to_string(),
                    }),
                    None => Err(SparrowError::RemoteError {
                        details: format!("unkown error with code: {code}"),
                    }),
                },
            },
        }
    }
}

extern crate self as sparrow_sdk;
// Compatibility alias so the `helix-dsl-macros` proc-macro (which emits
// `::helix_db::` paths) continues to resolve until the macro is updated.
extern crate self as helix_db;

#[cfg(test)]
mod tests {
    use sparrow_sdk::dsl::prelude::*;
    use std::collections::BTreeMap;

    #[register]
    fn query1(name: String) {
        // sparrow_sdk query that returns a read query or write query
        read_batch()
            .var_as("user", g().n_where(SourcePredicate::eq("username", name)))
            .var_as(
                "friends",
                g().n(NodeRef::var("user"))
                    .out(Some("FOLLOWS"))
                    .dedup()
                    .limit(100),
            )
            .returning(["user", "friends"])
    }

    #[test]
    fn query1_builds_dynamic_request() {
        // Calling the registered fn with concrete args yields a DynamicQueryRequest directly.
        let query = query1(String::from("alice"));

        assert!(matches!(query.request_type, DynamicQueryRequestType::Read));
        let params = query.parameters.expect("parameters present");
        assert!(matches!(
            params.get("name"),
            Some(DynamicQueryValue::String(s)) if s == "alice"
        ));
    }

    // ---- Group 1: every #[register] param type coerces correctly -----------

    #[register]
    fn q_bool(flag: bool) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", flag)))
            .returning(["v"])
    }
    #[register]
    fn q_i64(num: i64) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", num)))
            .returning(["v"])
    }
    #[register]
    fn q_f64(x: f64) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", x)))
            .returning(["v"])
    }
    #[register]
    fn q_f32(x: f32) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", x)))
            .returning(["v"])
    }
    #[register]
    fn q_datetime(ts: DateTime) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", ts)))
            .returning(["v"])
    }
    #[register]
    fn q_value(val: ParamValue) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", val)))
            .returning(["v"])
    }
    #[register]
    fn q_object(obj: ParamObject) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", obj)))
            .returning(["v"])
    }
    #[register]
    fn q_array(items: Vec<String>) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", items)))
            .returning(["v"])
    }
    #[register]
    fn q_map(map: BTreeMap<String, String>) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", map)))
            .returning(["v"])
    }
    #[register]
    #[allow(unused_variables)] // bytes coercion errors without reading the value (see test below)
    fn q_bytes(blob: Vec<u8>) {
        read_batch()
            .var_as("v", g().n_where(SourcePredicate::eq("field", blob)))
            .returning(["v"])
    }

    #[test]
    fn param_types_coerce_correctly() {
        // bool
        let r = q_bool(true);
        assert!(matches!(r.request_type, DynamicQueryRequestType::Read));
        assert!(matches!(
            r.parameters.as_ref().unwrap().get("flag"),
            Some(DynamicQueryValue::Bool(true))
        ));
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("flag"),
            Some(QueryParamType::Bool)
        ));

        // i64
        let r = q_i64(7);
        assert!(matches!(
            r.parameters.as_ref().unwrap().get("num"),
            Some(DynamicQueryValue::I64(7))
        ));
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("num"),
            Some(QueryParamType::I64)
        ));

        // f64
        let r = q_f64(1.5);
        assert!(matches!(
            r.parameters.as_ref().unwrap().get("x"),
            Some(DynamicQueryValue::F64(v)) if *v == 1.5
        ));
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("x"),
            Some(QueryParamType::F64)
        ));

        // f32
        let r = q_f32(1.5f32);
        assert!(matches!(
            r.parameters.as_ref().unwrap().get("x"),
            Some(DynamicQueryValue::F32(v)) if *v == 1.5f32
        ));
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("x"),
            Some(QueryParamType::F32)
        ));

        // DateTime -> rfc3339 string
        let r = q_datetime(DateTime::from_millis(0));
        let expected = DateTime::from_millis(0).to_rfc3339().unwrap();
        assert!(matches!(
            r.parameters.as_ref().unwrap().get("ts"),
            Some(DynamicQueryValue::String(s)) if *s == expected
        ));
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("ts"),
            Some(QueryParamType::DateTime)
        ));

        // ParamValue (PropertyValue)
        let r = q_value(PropertyValue::I64(5));
        assert!(matches!(
            r.parameters.as_ref().unwrap().get("val"),
            Some(DynamicQueryValue::I64(5))
        ));
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("val"),
            Some(QueryParamType::Value)
        ));

        // ParamObject (BTreeMap<String, PropertyValue>)
        let mut obj = BTreeMap::new();
        obj.insert("k".to_string(), PropertyValue::String("x".to_string()));
        let r = q_object(obj);
        assert!(matches!(
            r.parameters.as_ref().unwrap().get("obj"),
            Some(DynamicQueryValue::Object(_))
        ));
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("obj"),
            Some(QueryParamType::Object)
        ));

        // Vec<String> -> Array(String)
        let r = q_array(vec!["a".to_string(), "b".to_string()]);
        match r.parameters.as_ref().unwrap().get("items") {
            Some(DynamicQueryValue::Array(items)) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(&items[0], DynamicQueryValue::String(s) if s == "a"));
                assert!(matches!(&items[1], DynamicQueryValue::String(s) if s == "b"));
            }
            other => panic!("expected array, got {other:?}"),
        }
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("items"),
            Some(QueryParamType::Array(inner)) if matches!(**inner, QueryParamType::String)
        ));

        // BTreeMap<String, String> -> Object
        let mut map = BTreeMap::new();
        map.insert("k".to_string(), "v".to_string());
        let r = q_map(map);
        assert!(matches!(
            r.parameters.as_ref().unwrap().get("map"),
            Some(DynamicQueryValue::Object(_))
        ));
        assert!(matches!(
            r.parameter_types.as_ref().unwrap().get("map"),
            Some(QueryParamType::Object)
        ));
    }

    #[test]
    #[should_panic(expected = "failed to coerce parameter")]
    fn bytes_param_panics_on_dynamic_call() {
        // Bytes params register fine for the stored query, but dynamic coercion is unsupported
        // and the generated callable panics when invoked.
        let _ = q_bytes(vec![1, 2, 3]);
    }

    // ---- Group 2: SourcePredicate JSON — old (literal) vs new (param) -------

    #[test]
    fn source_predicate_literal_json_is_unchanged() {
        assert_eq!(
            sonic_rs::to_string(&SourcePredicate::eq("username", "alice")).unwrap(),
            r#"{"Eq":["username",{"String":"alice"}]}"#
        );
        assert_eq!(
            sonic_rs::to_string(&SourcePredicate::gt("score", 10i64)).unwrap(),
            r#"{"Gt":["score",{"I64":10}]}"#
        );
        assert_eq!(
            sonic_rs::to_string(&SourcePredicate::between("age", 18i64, 65i64)).unwrap(),
            r#"{"Between":["age",{"I64":18},{"I64":65}]}"#
        );
    }

    #[test]
    fn source_predicate_param_json_uses_expr_variants() {
        assert_eq!(
            sonic_rs::to_string(&SourcePredicate::eq("username", Expr::param("name"))).unwrap(),
            r#"{"EqExpr":["username",{"Param":"name"}]}"#
        );
        assert_eq!(
            sonic_rs::to_string(&SourcePredicate::lte("score", Expr::param("max"))).unwrap(),
            r#"{"LteExpr":["score",{"Param":"max"}]}"#
        );
        assert_eq!(
            sonic_rs::to_string(&SourcePredicate::between("age", Expr::param("lo"), 65i64))
                .unwrap(),
            r#"{"BetweenExpr":["age",{"Param":"lo"},{"Constant":{"I64":65}}]}"#
        );
    }

    #[test]
    fn source_predicate_json_round_trips() {
        for sp in [
            SourcePredicate::eq("username", "alice"),
            SourcePredicate::eq("username", Expr::param("name")),
            SourcePredicate::between("age", Expr::param("lo"), 65i64),
        ] {
            let json = sonic_rs::to_string(&sp).unwrap();
            let back: SourcePredicate = sonic_rs::from_str(&json).unwrap();
            assert_eq!(sp, back);
        }
    }

    // ---- Group 3: full query AST, literal vs param (self-contained) --------

    #[test]
    fn query_ast_literal_vs_param_json() {
        let literal = read_batch()
            .var_as(
                "user",
                g().n_where(SourcePredicate::eq("username", "alice")),
            )
            .returning(["user"]);
        let literal_json = sonic_rs::to_string(&literal).unwrap();
        assert!(
            literal_json.contains(r#"{"NWhere":{"Eq":["username",{"String":"alice"}]}}"#),
            "literal NWhere step changed shape: {literal_json}"
        );
        assert!(!literal_json.contains("EqExpr"));

        let param = read_batch()
            .var_as(
                "user",
                g().n_where(SourcePredicate::eq("username", Expr::param("name"))),
            )
            .returning(["user"]);
        let param_json = sonic_rs::to_string(&param).unwrap();
        assert!(
            param_json.contains(r#"{"NWhere":{"EqExpr":["username",{"Param":"name"}]}}"#),
            "param NWhere step missing EqExpr/Param: {param_json}"
        );
    }
}

#[cfg(test)]
mod client_tests {
    //! Tests for the `Client` / `QueryBuilder` request-building surface. These
    //! exercise everything up to (but not including) the network round-trip, so
    //! they need no running SparrowDB instance. As a child module of the crate root
    //! they can read the builder's private fields directly.
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Resp;

    fn sample_request() -> DynamicQueryRequest {
        DynamicQueryRequest::read(
            read_batch()
                .var_as(
                    "user",
                    g().n_where(SourcePredicate::eq("username", "alice")),
                )
                .returning(["user"]),
        )
    }

    // ---- Client construction ------------------------------------------------

    #[test]
    fn new_defaults_to_localhost() {
        let client = Client::new(None).unwrap();
        assert_eq!(client.url.as_str(), "http://localhost:6969/v1/query");
        assert!(client.api_key.is_none());
    }

    #[test]
    fn new_parses_custom_url() {
        let client = Client::new(Some("https://cluster.sparrowdb.example.com")).unwrap();
        assert_eq!(client.url.as_str(), "https://cluster.sparrowdb.example.com/v1/query");
    }

    #[test]
    fn new_rejects_invalid_url() {
        let err = Client::new(Some("not a url")).unwrap_err();
        assert!(matches!(err, SparrowError::InvalidURL(_)));
    }

    #[test]
    fn with_api_key_sets_and_clears() {
        let client = Client::new(None).unwrap().with_api_key(Some("hx_secret"));
        assert_eq!(client.api_key.as_deref(), Some("hx_secret"));

        let cleared = client.with_api_key(None);
        assert!(cleared.api_key.is_none());
    }

    // ---- Header assembly ----------------------------------------------------

    #[test]
    fn query_builder_starts_with_only_content_type() {
        let client = Client::new(None).unwrap();
        let builder = client.query::<Resp>();
        assert_eq!(
            builder.headers[0],
            Some(("Content-Type", "application/json"))
        );
        assert!(builder.headers[1..].iter().all(Option::is_none));
    }

    #[test]
    fn header_toggles_populate_slots() {
        let client = Client::new(None).unwrap();
        let builder = client
            .query::<Resp>()
            .writer_only()
            .warm_only()
            .should_await_durability(true);
        assert_eq!(builder.headers[1], Some(("x-helix-require-writer", "true")));
        assert_eq!(builder.headers[2], Some(("x-helix-warm", "true")));
        assert_eq!(builder.headers[3], Some(("x-helix-await-durable", "true")));
    }

    #[test]
    fn should_await_durability_false_sends_false() {
        let client = Client::new(None).unwrap();
        let builder = client.query::<Resp>().should_await_durability(false);
        assert_eq!(builder.headers[3], Some(("x-helix-await-durable", "false")));
    }

    // ---- Query type + body --------------------------------------------------

    #[test]
    fn dynamic_query_sets_query_type() {
        let client = Client::new(None).unwrap();
        let request = client.query::<Resp>().dynamic_query(sample_request());
        assert!(matches!(request.request.query_type, QueryType::Dynamic(_)));
    }

    #[test]
    fn stored_query_sets_query_type() {
        let client = Client::new(None).unwrap();
        let request = client.query::<Resp>().stored_query("add_user".to_string());
        assert!(
            matches!(&request.request.query_type, QueryType::Stored(name) if name == "add_user")
        );
    }

    #[derive(serde::Serialize)]
    struct Payload {
        name: String,
    }

    #[test]
    fn body_serializes_payload() {
        let client = Client::new(None).unwrap();
        let payload = Payload {
            name: "alice".to_string(),
        };
        let builder = client.query::<Resp>().body(&payload).unwrap();
        assert_eq!(builder.body, Some(sonic_rs::to_vec(&payload).unwrap()));
    }

    // ---- Request routing (exercises the real `send()` path) -----------------

    #[derive(serde::Deserialize)]
    struct EmptyResp {}

    /// Spawn a one-shot HTTP server on a random port. Returns its base URL and a
    /// handle that resolves to the request-target (path) of the first request.
    async fn spawn_capture_server() -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let request_line = String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .unwrap()
                .to_string();
            // `METHOD <target> HTTP/1.1` -> the target.
            let target = request_line.split_whitespace().nth(1).unwrap().to_string();
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
            socket.write_all(resp.as_bytes()).await.unwrap();
            target
        });
        (base, handle)
    }

    #[tokio::test]
    async fn dynamic_query_posts_to_v1_query() {
        let (base, handle) = spawn_capture_server().await;
        let client = Client::new(Some(&base)).unwrap();
        let _: EmptyResp = client
            .query()
            .dynamic_query(sample_request())
            .send()
            .await
            .unwrap();
        assert_eq!(handle.await.unwrap(), "/v1/query");
    }

    #[tokio::test]
    async fn stored_query_posts_to_named_route() {
        let (base, handle) = spawn_capture_server().await;
        let client = Client::new(Some(&base)).unwrap();
        let _: EmptyResp = client
            .query()
            .stored_query("add_user".to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(handle.await.unwrap(), "/v1/query/add_user");
    }
}
