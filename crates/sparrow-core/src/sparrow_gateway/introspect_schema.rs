use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;

use crate::sparrow_gateway::gateway::AppState;
use axum::response::IntoResponse;

pub async fn introspect_schema_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    #[cfg(feature = "lmdb")]
    {
        if state.token_store.is_auth_required() {
            let raw_key = headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if state.token_store.verify(raw_key).is_err() {
                use crate::protocol::SparrowError;
                return SparrowError::InvalidApiKey.into_response();
            }
        }
    }

    // Suppress unused variable warning when lmdb feature is disabled.
    let _ = &headers;

    match state.schema_json.as_ref() {
        Some(data) => axum::response::Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(data.clone()))
            .expect("should be able to make response from string"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Could not find schema").into_response(),
    }
}
