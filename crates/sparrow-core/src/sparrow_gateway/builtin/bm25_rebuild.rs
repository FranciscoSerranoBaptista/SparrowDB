use std::sync::Arc;

use tracing::{info, warn};

use crate::protocol;
use crate::sparrow_engine::types::GraphError;
use crate::sparrow_gateway::router::router::{Handler, HandlerInput, HandlerSubmission};
use crate::utils::items::Node;

// POST /rebuild-bm25-index
//
// Clears the BM25 full-text index and rebuilds it from scratch by iterating
// every node currently in the node database.
//
// Intended for use after a bulk import performed with SPARROW_SKIP_BM25_ON_WRITE=true,
// where inline BM25 updates were skipped to avoid write stall and OOM.
//
// The rebuild runs in chunks of 500 nodes, committing and force-syncing after each
// chunk so that dirty-page pressure stays bounded even for very large graphs.
//
// Response:
//   {"ok": true, "indexed": N, "total": M}
//
// Errors:
//   If BM25 is not enabled (bm25: false in config), returns an error.

pub fn rebuild_bm25_index_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    let bm25 = db.bm25.as_ref().ok_or_else(|| {
        GraphError::New("BM25 is not enabled (set bm25: true in config and restart)".to_string())
    })?;

    // Refuse to wipe and rebuild the index while skip_bm25_on_write is set.
    // The flag signals that BM25 is intentionally paused (e.g. during bulk import).
    // Silently proceeding would defeat the operator's intent: the entire existing
    // index would be cleared and then immediately rebuilt — the exact expensive
    // operation the flag is meant to defer.
    if db
        .skip_bm25_writes
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(GraphError::New(
            "BM25 rebuild rejected: skip_bm25_on_write is active. \
             Clear the flag via POST /settings before rebuilding."
                .to_string(),
        ));
    }

    let mut arena = bumpalo::Bump::new();

    // Phase 1: snapshot all node IDs + serialized bytes under a read transaction.
    // This avoids holding a write transaction open during the full-table scan.
    let node_data: Vec<(u128, Vec<u8>)> = {
        let rtxn = db.graph_env.read_txn().map_err(GraphError::from)?;
        db.nodes_db
            .iter(&rtxn)
            .map_err(GraphError::from)?
            .map(|r| r.map(|(id, bytes)| (id, bytes.to_vec())))
            .collect::<Result<Vec<_>, _>>()
            .map_err(GraphError::from)?
    };

    let total = node_data.len() as u64;
    info!(total, "BM25 rebuild: clearing existing index");

    // Phase 2: clear old BM25 data in its own transaction + sync
    {
        let mut wtxn = db.graph_env.write_txn().map_err(GraphError::from)?;
        bm25.clear_all(&mut wtxn)?;
        wtxn.commit().map_err(GraphError::from)?;
        if let Err(e) = db.graph_env.force_sync() {
            warn!("BM25 rebuild: force_sync after clear failed: {e}");
        }
    }

    // Phase 3: re-index in chunks of CHUNK_SIZE, committing + syncing after each chunk.
    // This bounds dirty-page accumulation to ~CHUNK_SIZE × avg_terms × entry_size.
    const CHUNK_SIZE: usize = 500;
    let mut indexed: u64 = 0;

    for (chunk_idx, chunk) in node_data.chunks(CHUNK_SIZE).enumerate() {
        let mut wtxn = db.graph_env.write_txn().map_err(GraphError::from)?;

        for (id, node_bytes) in chunk {
            let node = match Node::from_bincode_bytes(*id, node_bytes, &arena) {
                Ok(n) => n,
                Err(e) => {
                    warn!(node_id = ?id, "BM25 rebuild: failed to deserialize node, skipping: {e}");
                    continue;
                }
            };

            // Honour the label exclude-list even during a manual rebuild so that
            // a rebuild after configuration change is idempotent (excluded labels
            // are not re-indexed on the next rebuild_bm25_index call).
            if db.bm25_exclude_labels.contains(node.label) {
                arena.reset();
                continue;
            }

            if let Some(props) = node.properties.as_ref() {
                if let Err(e) = bm25.insert_doc_for_node(&mut wtxn, *id, props, node.label) {
                    warn!(node_id = ?id, "BM25 rebuild: failed to index node, skipping: {e}");
                } else {
                    indexed += 1;
                }
            }
            // Reset arena between nodes to reclaim property string allocations.
            // SAFETY: node does not escape the loop body.
            arena.reset();
        }

        wtxn.commit().map_err(GraphError::from)?;

        if let Err(e) = db.graph_env.force_sync() {
            warn!(
                chunk_idx,
                "BM25 rebuild: force_sync after chunk failed: {e}"
            );
        }

        info!(chunk_idx, indexed, total, "BM25 rebuild: chunk committed");
    }

    info!(indexed, total, "BM25 rebuild: complete");

    let body = format!(r#"{{"ok":true,"indexed":{indexed},"total":{total}}}"#);
    Ok(protocol::Response {
        body: body.into_bytes(),
        fmt: Default::default(),
    })
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("rebuild_bm25_index", rebuild_bm25_index_inner, true)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::{request::Request, request::RequestType, Format},
        sparrow_engine::{
            storage_core::version_info::VersionInfo,
            traversal_core::{
                config::Config,
                ops::{g::G, source::add_n::AddNAdapter},
                SparrowGraphEngine, SparrowGraphEngineOpts,
            },
        },
        sparrow_gateway::router::router::HandlerInput,
    };
    use axum::body::Bytes;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup_test_engine() -> (SparrowGraphEngine, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let opts = SparrowGraphEngineOpts {
            path: temp_dir.path().to_str().unwrap().to_string(),
            config: Config::default(),
            version_info: VersionInfo::default(),
            skip_bm25_on_write: None,
        };
        let engine = SparrowGraphEngine::new(opts).unwrap();
        (engine, temp_dir)
    }

    fn make_post_request() -> Request {
        Request {
            name: "rebuild_bm25_index".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        }
    }

    #[test]
    fn test_rebuild_bm25_empty_db() {
        let (engine, _temp_dir) = setup_test_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request(),
        };

        let result = rebuild_bm25_index_inner(input);
        assert!(result.is_ok(), "empty rebuild should succeed: {result:?}");

        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(body.contains("\"ok\":true"), "got: {body}");
        assert!(body.contains("\"indexed\":0"), "got: {body}");
        assert!(body.contains("\"total\":0"), "got: {body}");
    }

    #[test]
    fn test_rebuild_bm25_with_nodes() -> Result<(), Box<dyn std::error::Error>> {
        use crate::protocol::value::Value;
        use crate::utils::properties::ImmutablePropertiesMap;

        let (engine, _temp_dir) = setup_test_engine();

        // Insert two nodes with properties
        {
            let arena = bumpalo::Bump::new();
            let mut txn = engine.storage.graph_env.write_txn().unwrap();

            let props = vec![("content", Value::String("hello world foo bar".to_string()))];
            let props_map = ImmutablePropertiesMap::new(
                props.len(),
                props
                    .iter()
                    .map(|(k, v)| (arena.alloc_str(k) as &str, v.clone())),
                &arena,
            );
            let _ = G::new_mut(&engine.storage, &arena, &mut txn)
                .add_n(arena.alloc_str("Doc"), Some(props_map), None)
                .collect_to_obj()?;

            let props2 = vec![("content", Value::String("another document".to_string()))];
            let props_map2 = ImmutablePropertiesMap::new(
                props2.len(),
                props2
                    .iter()
                    .map(|(k, v)| (arena.alloc_str(k) as &str, v.clone())),
                &arena,
            );
            let _ = G::new_mut(&engine.storage, &arena, &mut txn)
                .add_n(arena.alloc_str("Doc"), Some(props_map2), None)
                .collect_to_obj()?;

            txn.commit().unwrap();
        }

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request(),
        };

        let result = rebuild_bm25_index_inner(input);
        assert!(
            result.is_ok(),
            "rebuild with nodes should succeed: {result:?}"
        );

        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(body.contains("\"ok\":true"), "got: {body}");
        assert!(
            body.contains("\"indexed\":2"),
            "expected 2 indexed, got: {body}"
        );
        assert!(body.contains("\"total\":2"), "got: {body}");
        Ok(())
    }

    #[test]
    fn test_rebuild_bm25_with_skip_env_var() -> Result<(), Box<dyn std::error::Error>> {
        use crate::protocol::value::Value;
        use crate::utils::properties::ImmutablePropertiesMap;

        // Simulate bulk import: set env var so storage skips BM25 writes
        unsafe {
            std::env::set_var("SPARROW_SKIP_BM25_ON_WRITE", "true");
        }
        let (engine, _temp_dir) = setup_test_engine();
        // skip_bm25_writes=true at this point; verify it's set
        assert!(
            engine.storage.skip_bm25_writes.load(Ordering::Relaxed),
            "skip flag should be set from env var"
        );
        unsafe {
            std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE");
        }

        {
            let arena = bumpalo::Bump::new();
            let mut txn = engine.storage.graph_env.write_txn().unwrap();

            let props = vec![(
                "content",
                Value::String("skipped during import".to_string()),
            )];
            let props_map = ImmutablePropertiesMap::new(
                props.len(),
                props
                    .iter()
                    .map(|(k, v)| (arena.alloc_str(k) as &str, v.clone())),
                &arena,
            );
            let _ = G::new_mut(&engine.storage, &arena, &mut txn)
                .add_n(arena.alloc_str("Doc"), Some(props_map), None)
                .collect_to_obj()?;
            txn.commit().unwrap();
        }

        // Clear the skip flag before rebuilding — this is the documented recovery
        // workflow:  (1) POST /settings skip=true → bulk import → (2) POST /settings
        // skip=false → (3) POST /rebuild_bm25_index.  The rebuild guard rejects
        // requests while skip is active to prevent the index becoming immediately
        // stale from concurrent writes that bypass BM25.
        engine
            .storage
            .skip_bm25_writes
            .store(false, std::sync::atomic::Ordering::Release);

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request(),
        };

        let result = rebuild_bm25_index_inner(input);
        assert!(
            result.is_ok(),
            "rebuild after clearing skip flag should succeed: {result:?}"
        );

        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(body.contains("\"indexed\":1"), "got: {body}");
        Ok(())
    }
}
