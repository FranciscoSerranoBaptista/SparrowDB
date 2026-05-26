use std::sync::Arc;

use heed3::CompactionOption;
use tracing::{info, warn};

use crate::protocol;
use crate::sparrow_engine::types::GraphError;
use crate::sparrow_gateway::router::router::{Handler, HandlerInput, HandlerSubmission};

// POST /compact
//
// Writes a compacted copy of the LMDB data file alongside the live file.
// LMDB accumulates free pages after BM25 rebuilds (clear + rebuild) and
// general updates.  The compacted copy removes those dead pages and can
// reduce the file by 30-50 % on a graph that has had multiple BM25 rebuilds.
//
// This operation uses LMDB's built-in consistent-copy mechanism: it opens
// an internal read transaction so the output is a crash-safe point-in-time
// snapshot.  Reads and writes continue uninterrupted during the copy.
//
// Output file: <data_dir>/data.mdb.compact
// (Replace the live data.mdb with this file while the server is stopped to
//  reclaim the space.  The live file is never modified by this endpoint.)
//
// Response (success):
//   {"ok":true,"original_bytes":N,"compact_bytes":M,"compact_path":"..."}
//
// The compact_path is an absolute path inside the container / process working
// directory.  On the next planned maintenance window:
//   1. docker stop <container>
//   2. mv data.mdb data.mdb.pre-compact && mv data.mdb.compact data.mdb
//   3. docker start <container>
//
// Errors:
//   Returns an error if the compact file already exists (delete it first) or
//   if the underlying mdb_env_copyfd2 system call fails.

pub fn compact_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    let env_path = db.graph_env.path().to_owned();
    let original_path = env_path.join("data.mdb");
    let compact_path = env_path.join("data.mdb.compact");

    // Refuse to silently overwrite an existing compact file — the operator
    // may need to inspect or swap the previous one first.
    if compact_path.exists() {
        return Err(GraphError::New(format!(
            "compact file already exists at {}; delete it before running /compact again",
            compact_path.display()
        )));
    }

    let original_bytes = original_path.metadata().map(|m| m.len()).unwrap_or(0);

    info!(
        original_bytes,
        compact_path = %compact_path.display(),
        "compact: starting LMDB compacted copy"
    );

    db.graph_env
        .copy_to_path(&compact_path, CompactionOption::Enabled)
        .map_err(|e| {
            // Remove a partial file so the next attempt can start clean.
            if compact_path.exists() {
                if let Err(rm_err) = std::fs::remove_file(&compact_path) {
                    warn!(
                        path = %compact_path.display(),
                        "compact: failed to remove partial compact file after error: {rm_err}"
                    );
                }
            }
            GraphError::New(format!("compact: mdb_env_copy failed: {e}"))
        })?;

    let compact_bytes = compact_path.metadata().map(|m| m.len()).unwrap_or(0);

    let saved_bytes = original_bytes.saturating_sub(compact_bytes);
    let saved_pct = if original_bytes > 0 {
        saved_bytes * 100 / original_bytes
    } else {
        0
    };

    info!(
        original_bytes,
        compact_bytes,
        saved_bytes,
        saved_pct,
        compact_path = %compact_path.display(),
        "compact: done"
    );

    let body = format!(
        r#"{{"ok":true,"original_bytes":{original_bytes},"compact_bytes":{compact_bytes},"saved_bytes":{saved_bytes},"saved_pct":{saved_pct},"compact_path":"{}"}}"#,
        compact_path.display()
    );

    Ok(protocol::Response {
        body: body.into_bytes(),
        fmt: Default::default(),
    })
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("compact", compact_inner, true)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::{request::Request, request::RequestType, Format},
        sparrow_engine::{
            storage_core::version_info::VersionInfo,
            traversal_core::{config::Config, SparrowGraphEngine, SparrowGraphEngineOpts},
        },
        sparrow_gateway::router::router::HandlerInput,
    };
    use axum::body::Bytes;
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
            name: "compact".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            pre_computed_embedding: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
        }
    }

    #[test]
    fn test_compact_creates_compact_file() {
        let (engine, temp_dir) = setup_test_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request(),
        };

        let result = compact_inner(input);
        assert!(
            result.is_ok(),
            "compact should succeed on a fresh DB: {result:?}"
        );

        let compact_path = temp_dir.path().join("data.mdb.compact");
        assert!(compact_path.exists(), "data.mdb.compact must be created");
        assert!(
            compact_path.metadata().unwrap().len() > 0,
            "compact file must not be empty"
        );

        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(
            body.contains("\"ok\":true"),
            "response must contain ok:true"
        );
        assert!(
            body.contains("compact_bytes"),
            "response must report compact_bytes"
        );
    }

    #[test]
    fn test_compact_fails_when_compact_file_already_exists() {
        let (engine, temp_dir) = setup_test_engine();

        // Pre-create the compact file to simulate a previous run.
        let compact_path = temp_dir.path().join("data.mdb.compact");
        std::fs::write(&compact_path, b"placeholder").unwrap();

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_post_request(),
        };

        let result = compact_inner(input);
        assert!(
            result.is_err(),
            "should error when compact file already exists"
        );
        let msg = format!("{:?}", result.err().unwrap());
        assert!(
            msg.contains("already exists"),
            "error should mention existing file: {msg}"
        );
    }
}
