#[cfg(feature = "lmdb")]
use std::sync::Arc;

#[cfg(feature = "lmdb")]
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::HeaderMap,
    http::StatusCode,
    response::IntoResponse,
};
#[cfg(feature = "lmdb")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "lmdb")]
use crate::{
    protocol::SparrowError,
    sparrow_gateway::{
        auth::{Role, TokenRecord},
        gateway::AppState,
    },
};

#[cfg(feature = "lmdb")]
#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub role: RoleInput,
}

#[cfg(feature = "lmdb")]
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleInput {
    Admin,
    ReadWrite,
    ReadOnly,
}

#[cfg(feature = "lmdb")]
impl From<RoleInput> for Role {
    fn from(r: RoleInput) -> Self {
        match r {
            RoleInput::Admin => Role::Admin,
            RoleInput::ReadWrite => Role::ReadWrite,
            RoleInput::ReadOnly => Role::ReadOnly,
        }
    }
}

#[cfg(feature = "lmdb")]
#[derive(Serialize)]
pub struct CreateTokenResponse {
    pub token: String,
    pub record: TokenRecord,
}

/// Verify the caller holds an Admin token.
#[cfg(feature = "lmdb")]
pub fn require_admin(record: &TokenRecord) -> Result<(), SparrowError> {
    if record.role == Role::Admin {
        Ok(())
    } else {
        Err(SparrowError::Forbidden)
    }
}

#[cfg(feature = "lmdb")]
pub(crate) fn extract_verified_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TokenRecord, axum::http::Response<Body>> {
    // When no tokens exist, auth is disabled server-wide — allow bootstrap.
    if !state.token_store.is_auth_required() {
        return Ok(TokenRecord {
            id: "bootstrap".to_string(),
            name: "bootstrap".to_string(),
            role: Role::Admin,
            created_at: 0,
        });
    }
    let raw_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let record = state
        .token_store
        .verify(raw_key)
        .map_err(|_| SparrowError::InvalidApiKey.into_response())?;
    require_admin(&record).map_err(|e| e.into_response())?;
    Ok(record)
}

#[cfg(feature = "lmdb")]
pub async fn list_tokens_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(e) = extract_verified_admin(&state, &headers) {
        return e;
    }
    match state.token_store.list() {
        Ok(records) => Json(records).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(feature = "lmdb")]
pub async fn create_token_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateTokenRequest>,
) -> axum::response::Response {
    if let Err(e) = extract_verified_admin(&state, &headers) {
        return e;
    }
    match state.token_store.create(&body.name, body.role.into()) {
        Ok((raw_token, record)) => {
            Json(CreateTokenResponse { token: raw_token, record }).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(feature = "lmdb")]
pub async fn revoke_token_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
    if let Err(e) = extract_verified_admin(&state, &headers) {
        return e;
    }
    match state.token_store.revoke(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(all(test, feature = "lmdb"))]
mod tests {
    use super::*;
    use crate::sparrow_gateway::auth::{Role, TokenRecord};
    use crate::protocol::SparrowError;

    fn make_record(role: Role) -> TokenRecord {
        TokenRecord {
            id: "aabbccdd".to_string(),
            name: "test".to_string(),
            role,
            created_at: 0,
        }
    }

    #[test]
    fn test_require_admin_passes_for_admin_role() {
        let record = make_record(Role::Admin);
        assert!(require_admin(&record).is_ok());
    }

    #[test]
    fn test_require_admin_rejects_read_write_role() {
        let record = make_record(Role::ReadWrite);
        assert!(matches!(require_admin(&record), Err(SparrowError::Forbidden)));
    }

    #[test]
    fn test_require_admin_rejects_read_only_role() {
        let record = make_record(Role::ReadOnly);
        assert!(matches!(require_admin(&record), Err(SparrowError::Forbidden)));
    }
}
