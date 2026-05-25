use std::sync::Arc;

use crate::sparrow_gateway::{
    gateway::CoreSetter, router::router::SparrowRouter, settings::RuntimeSettings,
    worker_pool::WorkerPool,
};
use crate::{
    sparrow_engine::{
        storage_core::version_info::VersionInfo,
        traversal_core::{SparrowGraphEngine, SparrowGraphEngineOpts, config::Config},
    },
    sparrow_gateway::{gateway::AppState, introspect_schema::introspect_schema_handler},
};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use reqwest::StatusCode;
use tempfile::TempDir;
#[cfg(feature = "lmdb")]
use crate::sparrow_gateway::auth::TokenStore;

fn create_test_app_state(schema_json: Option<String>) -> Arc<AppState> {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().to_str().unwrap();
    let opts = SparrowGraphEngineOpts {
        path: db_path.to_string(),
        config: Config::default(),
        version_info: VersionInfo::default(),
        skip_bm25_on_write: None,
    };
    let graph = Arc::new(SparrowGraphEngine::new(opts).unwrap());
    let router = Arc::new(SparrowRouter::new(None, None, None));
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap(),
    );

    let cores = core_affinity::get_core_ids().unwrap_or_default();
    let core_setter = Arc::new(CoreSetter::new(cores, 2));
    let worker_pool = WorkerPool::new(core_setter, graph, router, rt);

    Arc::new(AppState {
        worker_pool,
        schema_json: schema_json.map(Bytes::from),
        cluster_id: None,
        settings: Arc::new(RuntimeSettings::from_env()),
        #[cfg(feature = "lmdb")]
        token_store: {
            let rnd: u64 = rand::random();
            Arc::new(TokenStore::open(&format!("/tmp/sparrow_auth_{rnd:x}")).unwrap())
        },
    })
}

fn empty_headers() -> HeaderMap {
    HeaderMap::new()
}

// ============================================================================
// Tests (no tokens seeded → auth disabled → handler passes through to schema)
// ============================================================================

#[tokio::test]
async fn test_introspect_schema_with_valid_schema() {
    let schema_json = r#"{"version":"1.0","tables":[]}"#.to_string();
    let state = create_test_app_state(Some(schema_json.clone()));

    let response = introspect_schema_handler(State(state), empty_headers()).await;

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response.headers().get("Content-Type");
    assert!(content_type.is_some());
    assert_eq!(content_type.unwrap(), "application/json");

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert_eq!(body_str, schema_json);
}

#[tokio::test]
async fn test_introspect_schema_without_schema() {
    let state = create_test_app_state(None);

    let response = introspect_schema_handler(State(state), empty_headers()).await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert_eq!(body_str, "Could not find schema");
}

#[tokio::test]
async fn test_introspect_schema_with_empty_schema() {
    let schema_json = "".to_string();
    let state = create_test_app_state(Some(schema_json.clone()));

    let response = introspect_schema_handler(State(state), empty_headers()).await;

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert_eq!(body_str, "");
}

#[tokio::test]
async fn test_introspect_schema_with_complex_schema() {
    let schema_json = r#"{"version":"2.0","tables":[{"name":"users","fields":["id","name","email"]},{"name":"posts","fields":["id","title","content"]}]}"#.to_string();
    let state = create_test_app_state(Some(schema_json.clone()));

    let response = introspect_schema_handler(State(state), empty_headers()).await;

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert_eq!(body_str, schema_json);
}

#[tokio::test]
async fn test_introspect_schema_response_format() {
    let schema_json = r#"{"test":"data"}"#.to_string();
    let state = create_test_app_state(Some(schema_json));

    let response = introspect_schema_handler(State(state), empty_headers()).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("Content-Type").unwrap(),
        "application/json"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(!body_bytes.is_empty());
}

// ============================================================================
// Auth-enforced tests (lmdb + a token seeded)
// ============================================================================

#[cfg(feature = "lmdb")]
#[tokio::test]
async fn test_introspect_schema_missing_api_key_when_auth_required() {
    use crate::sparrow_gateway::auth::Role;

    let schema_json = r#"{"version":"1.0","tables":[]}"#.to_string();
    let state = create_test_app_state(Some(schema_json));
    // Seed a token so auth becomes required.
    state.token_store.create("test-token", Role::ReadOnly).unwrap();

    // No x-api-key header → should return 401.
    let response = introspect_schema_handler(State(state), empty_headers()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[cfg(feature = "lmdb")]
#[tokio::test]
async fn test_introspect_schema_invalid_api_key_when_auth_required() {
    use crate::sparrow_gateway::auth::Role;

    let schema_json = r#"{"version":"1.0","tables":[]}"#.to_string();
    let state = create_test_app_state(Some(schema_json));
    state.token_store.create("test-token", Role::ReadOnly).unwrap();

    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", "invalid-key".parse().unwrap());
    let response = introspect_schema_handler(State(state), headers).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[cfg(feature = "lmdb")]
#[tokio::test]
async fn test_introspect_schema_valid_api_key_when_auth_required() {
    use crate::sparrow_gateway::auth::Role;

    let schema_json = r#"{"version":"1.0","tables":[]}"#.to_string();
    let state = create_test_app_state(Some(schema_json.clone()));
    let (raw_token, _) = state.token_store.create("test-token", Role::ReadOnly).unwrap();

    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", raw_token.parse().unwrap());
    let response = introspect_schema_handler(State(state), headers).await;
    assert_eq!(response.status(), StatusCode::OK);
}
