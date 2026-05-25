use std::sync::Arc;

use crate::sparrow_engine::types::GraphError;
use crate::sparrow_engine::vector_core::HNSW;
use crate::sparrow_gateway::router::router::{Handler, HandlerInput, HandlerSubmission};
use crate::protocol;
use sonic_rs::{JsonValueTrait, json};

// POST /vector-soft-delete
// Body: {"id": "<u128-as-string>"}
// Marks the vector as deleted (soft delete, HNSW graph structure preserved).

pub fn vector_soft_delete_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    let id_str = if !input.request.body.is_empty() {
        match sonic_rs::from_slice::<sonic_rs::Value>(&input.request.body) {
            Ok(params) => params
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            Err(_) => None,
        }
    } else {
        None
    };

    let id_str = id_str.ok_or_else(|| GraphError::New("id is required".to_string()))?;

    let id: u128 = match uuid::Uuid::parse_str(&id_str) {
        Ok(uuid) => uuid.as_u128(),
        Err(_) => match id_str.parse::<u128>() {
            Ok(v) => v,
            Err(_) => {
                return Err(GraphError::New(
                    "invalid ID format: must be UUID or u128".to_string(),
                ));
            }
        },
    };

    let mut txn = db.graph_env.write_txn()?;
    let arena = bumpalo::Bump::new();

    db.vectors.delete(&mut txn, id, &arena)?;

    txn.commit()?;

    let result = json!({ "ok": true, "id": id_str });
    Ok(protocol::Response {
        body: sonic_rs::to_vec(&result).map_err(|e| GraphError::New(e.to_string()))?,
        fmt: Default::default(),
    })
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("vector_soft_delete", vector_soft_delete_inner, true)
    )
}

// POST /vector-hard-delete
// Body: {"id": "<u128-as-string>"}
// Physically removes all HNSW data for the vector (hard delete).

pub fn vector_hard_delete_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    let id_str = if !input.request.body.is_empty() {
        match sonic_rs::from_slice::<sonic_rs::Value>(&input.request.body) {
            Ok(params) => params
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            Err(_) => None,
        }
    } else {
        None
    };

    let id_str = id_str.ok_or_else(|| GraphError::New("id is required".to_string()))?;

    let id: u128 = match uuid::Uuid::parse_str(&id_str) {
        Ok(uuid) => uuid.as_u128(),
        Err(_) => match id_str.parse::<u128>() {
            Ok(v) => v,
            Err(_) => {
                return Err(GraphError::New(
                    "invalid ID format: must be UUID or u128".to_string(),
                ));
            }
        },
    };

    let mut txn = db.graph_env.write_txn()?;

    db.vectors.hard_delete(&mut txn, id)?;

    txn.commit()?;

    let result = json!({
        "ok": true,
        "id": id_str,
        "warning": "HNSW graph structurally modified; consider POST /rebuild-vector-index"
    });
    Ok(protocol::Response {
        body: sonic_rs::to_vec(&result).map_err(|e| GraphError::New(e.to_string()))?,
        fmt: Default::default(),
    })
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("vector_hard_delete", vector_hard_delete_inner, true)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sparrow_engine::{
            storage_core::version_info::VersionInfo,
            traversal_core::{
                SparrowGraphEngine, SparrowGraphEngineOpts,
                config::Config,
            },
            vector_core::{HNSW, vector::HVector},
        },
        sparrow_gateway::router::router::HandlerInput,
        protocol::{Format, request::Request, request::RequestType},
    };
    use axum::body::Bytes;
    use bumpalo::Bump;
    use heed3::RoTxn;
    use std::sync::Arc;
    use tempfile::TempDir;

    type Filter = fn(&HVector, &RoTxn) -> bool;

    fn setup_test_engine() -> (SparrowGraphEngine, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();
        let opts = SparrowGraphEngineOpts {
            path: db_path.to_string(),
            config: Config::default(),
            version_info: VersionInfo::default(),
            skip_bm25_on_write: None,
        };
        let engine = SparrowGraphEngine::new(opts).unwrap();
        (engine, temp_dir)
    }

    fn insert_test_vector(engine: &SparrowGraphEngine) -> u128 {
        let mut txn = engine.storage.graph_env.write_txn().unwrap();
        let arena = Bump::new();
        let data = arena.alloc_slice_copy(&[0.1f64, 0.2, 0.3, 0.4]);
        let v = engine
            .storage
            .vectors
            .insert::<Filter>(&mut txn, "vector", data, None, &arena)
            .unwrap();
        let id = v.id;
        txn.commit().unwrap();
        id
    }

    #[test]
    fn test_vector_soft_delete_success() {
        let (engine, _temp_dir) = setup_test_engine();
        let id = insert_test_vector(&engine);
        let id_str = id.to_string();

        let params_json = sonic_rs::to_vec(&json!({"id": id_str})).unwrap();
        let request = Request {
            name: "vector_soft_delete".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::from(params_json),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        };

        let input = HandlerInput {
            graph: Arc::new(engine),
            request,
        };

        let result = vector_soft_delete_inner(input);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"ok\":true"));
        assert!(body_str.contains(&id_str));
    }

    #[test]
    fn test_vector_soft_delete_missing_id() {
        let (engine, _temp_dir) = setup_test_engine();

        let request = Request {
            name: "vector_soft_delete".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        };

        let input = HandlerInput {
            graph: Arc::new(engine),
            request,
        };

        let result = vector_soft_delete_inner(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_vector_soft_delete_invalid_id() {
        let (engine, _temp_dir) = setup_test_engine();

        let params_json = sonic_rs::to_vec(&json!({"id": "not-a-valid-id"})).unwrap();
        let request = Request {
            name: "vector_soft_delete".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::from(params_json),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        };

        let input = HandlerInput {
            graph: Arc::new(engine),
            request,
        };

        let result = vector_soft_delete_inner(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_vector_hard_delete_success() {
        let (engine, _temp_dir) = setup_test_engine();
        let id = insert_test_vector(&engine);
        let id_str = id.to_string();

        let params_json = sonic_rs::to_vec(&json!({"id": id_str})).unwrap();
        let request = Request {
            name: "vector_hard_delete".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::from(params_json),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        };

        let input = HandlerInput {
            graph: Arc::new(engine),
            request,
        };

        let result = vector_hard_delete_inner(input);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"ok\":true"));
        assert!(body_str.contains(&id_str));
        assert!(body_str.contains("warning"));
    }

    #[test]
    fn test_vector_hard_delete_missing_id() {
        let (engine, _temp_dir) = setup_test_engine();

        let request = Request {
            name: "vector_hard_delete".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        };

        let input = HandlerInput {
            graph: Arc::new(engine),
            request,
        };

        let result = vector_hard_delete_inner(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_vector_hard_delete_invalid_id() {
        let (engine, _temp_dir) = setup_test_engine();

        let params_json = sonic_rs::to_vec(&json!({"id": "not-a-valid-id"})).unwrap();
        let request = Request {
            name: "vector_hard_delete".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::from(params_json),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        };

        let input = HandlerInput {
            graph: Arc::new(engine),
            request,
        };

        let result = vector_hard_delete_inner(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_vector_hard_delete_accepts_uuid_format() {
        let (engine, _temp_dir) = setup_test_engine();
        let id = insert_test_vector(&engine);

        // Express the id as a UUID string
        let uuid_str = uuid::Uuid::from_u128(id).to_string();
        let params_json = sonic_rs::to_vec(&json!({"id": uuid_str})).unwrap();
        let request = Request {
            name: "vector_hard_delete".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::from(params_json),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        };

        let input = HandlerInput {
            graph: Arc::new(engine),
            request,
        };

        let result = vector_hard_delete_inner(input);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn test_vector_soft_delete_accepts_uuid_format() {
        let (engine, _temp_dir) = setup_test_engine();
        let id = insert_test_vector(&engine);

        // Express the id as a UUID string
        let uuid_str = uuid::Uuid::from_u128(id).to_string();
        let params_json = sonic_rs::to_vec(&json!({"id": uuid_str})).unwrap();
        let request = Request {
            name: "vector_soft_delete".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::from(params_json),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        };

        let input = HandlerInput {
            graph: Arc::new(engine),
            request,
        };

        let result = vector_soft_delete_inner(input);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }
}
