use std::sync::Arc;

use crate::sparrow_engine::types::GraphError;
use crate::sparrow_gateway::router::router::{Handler, HandlerInput, HandlerSubmission};
use crate::protocol;

// POST /rebuild-vector-index
// Clears all HNSW data and re-inserts every non-deleted vector with its original ID.
// Soft-deleted vectors are permanently removed.
//
// Response: {"ok": true, "kept": N, "purged_deleted": M}

pub fn rebuild_vector_index_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    let mut txn = db.graph_env.write_txn()?;
    let arena = bumpalo::Bump::new();

    let stats = db
        .vectors
        .rebuild(&mut txn, &arena)
        .map_err(|e| GraphError::New(e.to_string()))?;

    txn.commit()?;

    let body = format!(
        r#"{{"ok":true,"kept":{kept},"purged_deleted":{purged}}}"#,
        kept = stats.kept,
        purged = stats.purged_deleted,
    );

    Ok(protocol::Response {
        body: body.into_bytes(),
        fmt: Default::default(),
    })
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("rebuild_vector_index", rebuild_vector_index_inner, true)
    )
}

// POST /purge-soft-deleted
// Alias for rebuild-vector-index: removes soft-deleted vectors from the HNSW index.
//
// Response: {"ok": true, "purged": N, "remaining": M}

pub fn purge_soft_deleted_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    let mut txn = db.graph_env.write_txn()?;
    let arena = bumpalo::Bump::new();

    let stats = db
        .vectors
        .purge_soft_deleted(&mut txn, &arena)
        .map_err(|e| GraphError::New(e.to_string()))?;

    txn.commit()?;

    let body = format!(
        r#"{{"ok":true,"purged":{purged},"remaining":{remaining}}}"#,
        purged = stats.purged_deleted,
        remaining = stats.kept,
    );

    Ok(protocol::Response {
        body: body.into_bytes(),
        fmt: Default::default(),
    })
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("purge_soft_deleted", purge_soft_deleted_inner, true)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sparrow_engine::{
            storage_core::version_info::VersionInfo,
            traversal_core::{SparrowGraphEngine, SparrowGraphEngineOpts, config::Config},
            vector_core::HNSW,
        },
        sparrow_gateway::router::router::HandlerInput,
        protocol::{Format, request::Request, request::RequestType},
    };
    use axum::body::Bytes;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup_test_engine() -> (SparrowGraphEngine, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();
        let opts = SparrowGraphEngineOpts {
            path: db_path.to_string(),
            config: Config::default(),
            version_info: VersionInfo::default(),
        };
        let engine = SparrowGraphEngine::new(opts).unwrap();
        (engine, temp_dir)
    }

    fn make_post_request(name: &str) -> Request {
        Request {
            name: name.to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        }
    }

    #[test]
    fn test_rebuild_vector_index_empty_db() {
        let (engine, _temp_dir) = setup_test_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request("rebuild_vector_index"),
        };

        let result = rebuild_vector_index_inner(input);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"ok\":true"));
        assert!(body_str.contains("\"kept\":0"));
        assert!(body_str.contains("\"purged_deleted\":0"));
    }

    #[test]
    fn test_rebuild_vector_index_with_deleted_vectors() {
        let (engine, _temp_dir) = setup_test_engine();

        // Insert two vectors and soft-delete one
        {
            let mut txn = engine.storage.graph_env.write_txn().unwrap();
            let arena = bumpalo::Bump::new();

            let v1 = engine
                .storage
                .vectors
                .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[1.0f64, 0.0, 0.0], None, &arena)
                .unwrap();
            let _v2 = engine
                .storage
                .vectors
                .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[0.0f64, 1.0, 0.0], None, &arena)
                .unwrap();

            engine
                .storage
                .vectors
                .delete(&mut txn, v1.id, &arena)
                .unwrap();
            txn.commit().unwrap();
        }

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request("rebuild_vector_index"),
        };

        let result = rebuild_vector_index_inner(input);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"ok\":true"));
        assert!(body_str.contains("\"kept\":1"));
        assert!(body_str.contains("\"purged_deleted\":1"));
    }

    #[test]
    fn test_purge_soft_deleted_empty_db() {
        let (engine, _temp_dir) = setup_test_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request("purge_soft_deleted"),
        };

        let result = purge_soft_deleted_inner(input);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"ok\":true"));
        assert!(body_str.contains("\"purged\":0"));
        assert!(body_str.contains("\"remaining\":0"));
    }

    #[test]
    fn test_purge_soft_deleted_with_deleted_vectors() {
        let (engine, _temp_dir) = setup_test_engine();

        {
            let mut txn = engine.storage.graph_env.write_txn().unwrap();
            let arena = bumpalo::Bump::new();

            let v1 = engine
                .storage
                .vectors
                .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[1.0f64, 0.0, 0.0], None, &arena)
                .unwrap();
            let _v2 = engine
                .storage
                .vectors
                .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &[0.0f64, 1.0, 0.0], None, &arena)
                .unwrap();

            engine
                .storage
                .vectors
                .delete(&mut txn, v1.id, &arena)
                .unwrap();
            txn.commit().unwrap();
        }

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request("purge_soft_deleted"),
        };

        let result = purge_soft_deleted_inner(input);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"ok\":true"));
        assert!(body_str.contains("\"purged\":1"));
        assert!(body_str.contains("\"remaining\":1"));
    }
}
