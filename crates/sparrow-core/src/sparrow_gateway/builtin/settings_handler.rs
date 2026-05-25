// crates/sparrow-core/src/sparrow_gateway/builtin/settings_handler.rs

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::{
    protocol::SparrowError,
    sparrow_gateway::{
        gateway::AppState,
        settings::RuntimeSettings,
    },
};

/// Apply a partial settings patch from a JSON object.
///
/// Returns `Ok(())` on success, `Err(message)` on validation failure.
/// Pure function — no HTTP concerns — making it testable without axum.
pub fn apply_settings_patch(
    settings: &Arc<RuntimeSettings>,
    patch: &serde_json::Value,
) -> Result<(), String> {
    let obj = patch
        .as_object()
        .ok_or_else(|| "request body must be a JSON object".to_string())?;

    for (key, value) in obj {
        match key.as_str() {
            "skip_bm25_on_write" => {
                let v = value
                    .as_bool()
                    .ok_or_else(|| "skip_bm25_on_write must be a boolean".to_string())?;
                settings.set_skip_bm25_on_write(v);
            }
            "worker_threads" => {
                return Err(
                    "setting 'worker_threads' is immutable — restart the container to change it"
                        .to_string(),
                );
            }
            other => {
                return Err(format!("unknown setting '{other}'"));
            }
        }
    }
    Ok(())
}

/// `GET /settings` — return all settings with value, source, and mutability.
/// Requires any authenticated role (or no auth if auth is disabled).
#[cfg(feature = "lmdb")]
pub async fn get_settings_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if state.token_store.is_auth_required() {
        let raw_key = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if state.token_store.verify(raw_key).is_err() {
            return SparrowError::InvalidApiKey.into_response();
        }
    }

    let json = state.settings.to_json();
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json,
    )
        .into_response()
}

/// `POST /settings` — apply partial update to mutable settings.
/// Requires Admin role.
#[cfg(feature = "lmdb")]
pub async fn post_settings_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use crate::sparrow_gateway::builtin::token_mgmt::extract_verified_admin;
    if let Err(e) = extract_verified_admin(&state, &headers) {
        return e;
    }

    let patch: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [("content-type", "application/json")],
                format!(r#"{{"error":"invalid JSON: {e}"}}"#),
            )
                .into_response();
        }
    };

    match apply_settings_patch(&state.settings, &patch) {
        Ok(()) => {
            let json = state.settings.to_json();
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                json,
            )
                .into_response()
        }
        Err(msg) => (
            StatusCode::BAD_REQUEST,
            [("content-type", "application/json")],
            format!(r#"{{"error":"{msg}"}}"#),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "lmdb"))]
pub async fn get_settings_handler() -> axum::response::Response {
    (StatusCode::NOT_IMPLEMENTED, "lmdb feature required").into_response()
}

#[cfg(not(feature = "lmdb"))]
pub async fn post_settings_handler() -> axum::response::Response {
    (StatusCode::NOT_IMPLEMENTED, "lmdb feature required").into_response()
}

#[cfg(test)]
mod handler_tests {
    use super::*;

    #[test]
    fn apply_patch_accepts_known_mutable_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        // SAFETY: test-only, single-threaded context
        unsafe { std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE") };
        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"skip_bm25_on_write": true});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_ok());
        assert!(settings.skip_bm25_on_write.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn apply_patch_rejects_immutable_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"worker_threads": 16});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("immutable"));
    }

    #[test]
    fn apply_patch_rejects_unknown_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"unknown_setting": true});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown setting"));
    }
}
