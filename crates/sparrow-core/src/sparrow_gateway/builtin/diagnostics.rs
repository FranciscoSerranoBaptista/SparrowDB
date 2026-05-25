use std::sync::Arc;

use crate::sparrow_engine::types::GraphError;
use crate::sparrow_gateway::mem_monitor;
use crate::sparrow_gateway::router::router::{Handler, HandlerInput, HandlerSubmission};
use crate::protocol;

// GET /diagnostics
// curl "http://localhost:PORT/diagnostics"
//
// Returns counts of nodes, edges, vector stats, and live system metrics:
// {
//   "nodes": 1234,
//   "edges": 567,
//   "vectors": {
//     "total": 100,
//     "active": 90,
//     "soft_deleted": 10,
//     "hnsw_edges": 500,
//     "entry_point_present": true
//   },
//   "system": {
//     "rss_kb": 524288,
//     "memory_limit_kb": 6291456,
//     "rss_pct": 8.3,
//     "thread_count": 66
//   }
// }
//
// `memory_limit_kb` and `rss_pct` are 0 outside a cgroup-constrained container
// or on non-Linux platforms.

pub fn diagnostics_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    #[cfg(feature = "lmdb")]
    {
        let txn = db.graph_env.read_txn().map_err(GraphError::from)?;

        let nodes = db.nodes_db.len(&txn).map_err(GraphError::from)?;
        let edges = db.edges_db.len(&txn).map_err(GraphError::from)?;

        let vector_stats = db
            .vectors
            .stats(&txn)
            .map_err(|e| GraphError::New(e.to_string()))?;

        // System metrics — point-in-time snapshot (cheap /proc reads on Linux)
        let rss_kb = mem_monitor::read_rss_kb();
        let limit_kb = mem_monitor::read_cgroup_limit_kb();
        let threads = mem_monitor::read_thread_count();
        let rss_pct = if limit_kb > 0 {
            rss_kb as f64 / limit_kb as f64 * 100.0
        } else {
            0.0
        };

        let body = format!(
            r#"{{"nodes":{nodes},"edges":{edges},"vectors":{{"total":{total},"active":{active},"soft_deleted":{soft_deleted},"hnsw_edges":{hnsw_edges},"entry_point_present":{entry_point_present}}},"system":{{"rss_kb":{rss_kb},"memory_limit_kb":{limit_kb},"rss_pct":{rss_pct:.1},"thread_count":{threads}}}}}"#,
            nodes = nodes,
            edges = edges,
            total = vector_stats.total,
            active = vector_stats.active,
            soft_deleted = vector_stats.soft_deleted,
            hnsw_edges = vector_stats.hnsw_edges,
            entry_point_present = vector_stats.entry_point_present,
            rss_kb = rss_kb,
            limit_kb = limit_kb,
            rss_pct = rss_pct,
            threads = threads,
        );

        return Ok(protocol::Response {
            body: body.into_bytes(),
            fmt: Default::default(),
        });
    }

    #[cfg(not(feature = "lmdb"))]
    {
        Err(GraphError::New(
            "diagnostics endpoint requires lmdb feature".to_string(),
        ))
    }
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("diagnostics", diagnostics_inner, false)
    )
}

// GET /hnsw-health
// curl "http://localhost:PORT/hnsw-health"
//
// Runs BFS from HNSW entry point (level 0) and reports unreachable active vectors.
// {"status":"healthy"|"degraded"|"broken","total_active":N,"reachable":R,"unreachable":U}
//
// healthy  = 0 unreachable
// degraded = 1–5% unreachable
// broken   = >5% unreachable, or no entry point when active vectors exist

pub fn hnsw_health_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    #[cfg(feature = "lmdb")]
    {
        let txn = db.graph_env.read_txn().map_err(GraphError::from)?;

        let vector_stats = db
            .vectors
            .stats(&txn)
            .map_err(|e| GraphError::New(e.to_string()))?;
        let total_active = vector_stats.active as usize;

        let reachable = db
            .vectors
            .bfs_reachable_count_global(&txn)
            .map_err(|e| GraphError::New(e.to_string()))?;

        let unreachable = total_active.saturating_sub(reachable);

        let status = if total_active == 0 || unreachable == 0 {
            "healthy"
        } else if (unreachable as f64 / total_active as f64) <= 0.05 {
            "degraded"
        } else {
            "broken"
        };

        let body = format!(
            r#"{{"status":"{status}","total_active":{total_active},"reachable":{reachable},"unreachable":{unreachable}}}"#,
        );

        return Ok(protocol::Response {
            body: body.into_bytes(),
            fmt: Default::default(),
        });
    }

    #[cfg(not(feature = "lmdb"))]
    {
        Err(GraphError::New(
            "hnsw-health endpoint requires lmdb feature".to_string(),
        ))
    }
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("hnsw_health", hnsw_health_inner, false)
    )
}

// GET /hnsw-integrity
// curl "http://localhost:PORT/hnsw-integrity"
//
// Scans every HNSW edge and verifies bidirectional symmetry: for every A→B,
// B→A must also exist. Asymmetric edges indicate a graph corruption.
//
// {"symmetric":true,"total_edges":1000,"asymmetric_edges":0}

pub fn hnsw_integrity_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    #[cfg(feature = "lmdb")]
    {
        let txn = db.graph_env.read_txn().map_err(GraphError::from)?;

        let (total_edges, asymmetric_edges) = db
            .vectors
            .count_asymmetric_edges(&txn)
            .map_err(|e| GraphError::New(e.to_string()))?;

        let symmetric = asymmetric_edges == 0;

        let body = format!(
            r#"{{"symmetric":{symmetric},"total_edges":{total_edges},"asymmetric_edges":{asymmetric_edges}}}"#,
        );

        return Ok(protocol::Response {
            body: body.into_bytes(),
            fmt: Default::default(),
        });
    }

    #[cfg(not(feature = "lmdb"))]
    {
        Err(GraphError::New(
            "hnsw-integrity endpoint requires lmdb feature".to_string(),
        ))
    }
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("hnsw_integrity", hnsw_integrity_inner, false)
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
                ops::{
                    g::G,
                    source::{add_e::AddEAdapter, add_n::AddNAdapter},
                },
            },
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
            skip_bm25_on_write: None,
        };
        let engine = SparrowGraphEngine::new(opts).unwrap();
        (engine, temp_dir)
    }

    fn make_request() -> Request {
        Request {
            name: "diagnostics".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
            pre_computed_embedding: None,
        }
    }

    #[test]
    fn test_diagnostics_empty_db() {
        let (engine, _temp_dir) = setup_test_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_request(),
        };

        let result = diagnostics_inner(input);
        assert!(result.is_ok());

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"nodes\":0"));
        assert!(body_str.contains("\"edges\":0"));
        assert!(body_str.contains("\"vectors\""));
        assert!(body_str.contains("\"total\":0"));
        assert!(body_str.contains("\"active\":0"));
        assert!(body_str.contains("\"soft_deleted\":0"));
        assert!(body_str.contains("\"hnsw_edges\":0"));
        assert!(body_str.contains("\"entry_point_present\":false"));
        // System section always present; rss_kb > 0 on Linux, 0 elsewhere
        assert!(body_str.contains("\"system\""), "diagnostics must include system section: {body_str}");
        assert!(body_str.contains("\"rss_kb\""), "diagnostics must include rss_kb: {body_str}");
        assert!(body_str.contains("\"memory_limit_kb\""), "diagnostics must include memory_limit_kb: {body_str}");
        assert!(body_str.contains("\"rss_pct\""), "diagnostics must include rss_pct: {body_str}");
        assert!(body_str.contains("\"thread_count\""), "diagnostics must include thread_count: {body_str}");
    }

    #[test]
    fn test_diagnostics_with_nodes_and_edges() -> Result<(), Box<dyn std::error::Error>> {
        use crate::protocol::value::Value;
        use crate::utils::properties::ImmutablePropertiesMap;

        let (engine, _temp_dir) = setup_test_engine();
        let mut txn = engine.storage.graph_env.write_txn().unwrap();
        let arena = bumpalo::Bump::new();

        let props1 = vec![("name", Value::String("Alice".to_string()))];
        let props_map1 = ImmutablePropertiesMap::new(
            props1.len(),
            props1
                .iter()
                .map(|(k, v)| (arena.alloc_str(k) as &str, v.clone())),
            &arena,
        );
        let node1 = G::new_mut(&engine.storage, &arena, &mut txn)
            .add_n(arena.alloc_str("person"), Some(props_map1), None)
            .collect_to_obj()?;

        let props2 = vec![("name", Value::String("Bob".to_string()))];
        let props_map2 = ImmutablePropertiesMap::new(
            props2.len(),
            props2
                .iter()
                .map(|(k, v)| (arena.alloc_str(k) as &str, v.clone())),
            &arena,
        );
        let node2 = G::new_mut(&engine.storage, &arena, &mut txn)
            .add_n(arena.alloc_str("person"), Some(props_map2), None)
            .collect_to_obj()?;

        let _edge = G::new_mut(&engine.storage, &arena, &mut txn)
            .add_edge(
                arena.alloc_str("knows"),
                None,
                node1.id(),
                node2.id(),
                false,
            )
            .collect_to_obj()?;

        txn.commit().unwrap();

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_request(),
        };

        let result = diagnostics_inner(input);
        assert!(result.is_ok());

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"nodes\":2"));
        assert!(body_str.contains("\"edges\":1"));
        Ok(())
    }

    fn make_hnsw_health_request() -> Request {
        Request {
            name: "hnsw_health".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
            pre_computed_embedding: None,
        }
    }

    #[test]
    fn test_hnsw_health_empty_db_is_healthy() {
        let (engine, _temp_dir) = setup_test_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_hnsw_health_request(),
        };

        let result = hnsw_health_inner(input);
        assert!(result.is_ok(), "hnsw_health on empty db should succeed: {result:?}");

        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(body.contains("\"status\":\"healthy\""), "empty db should be healthy, got: {body}");
        assert!(body.contains("\"unreachable\":0"), "got: {body}");
    }

    #[test]
    fn test_hnsw_health_after_inserts_is_healthy() -> Result<(), Box<dyn std::error::Error>> {
        use crate::sparrow_engine::vector_core::HNSW;

        let (engine, _temp_dir) = setup_test_engine();
        let arena = bumpalo::Bump::new();
        let mut txn = engine.storage.graph_env.write_txn().unwrap();

        for i in 0..10i64 {
            let data = vec![i as f64 + 1.0, 0.0, 0.0, 0.0]; // non-zero vectors
            engine.storage.vectors
                .insert::<fn(&_, &_) -> bool>(&mut txn, "default", &data, None, &arena)
                .unwrap();
        }
        txn.commit().unwrap();

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_hnsw_health_request(),
        };

        let result = hnsw_health_inner(input)?;
        let body = String::from_utf8(result.body).unwrap();
        assert!(body.contains("\"status\":\"healthy\""), "10 inserts should be healthy, got: {body}");
        assert!(body.contains("\"total_active\":10"), "got: {body}");
        assert!(body.contains("\"unreachable\":0"), "got: {body}");
        Ok(())
    }

    #[test]
    fn test_diagnostics_with_vectors() -> Result<(), Box<dyn std::error::Error>> {
        use crate::sparrow_engine::vector_core::HNSW;

        let (engine, _temp_dir) = setup_test_engine();
        let arena = bumpalo::Bump::new();
        let mut txn = engine.storage.graph_env.write_txn().unwrap();

        let v1_data = vec![1.0f64, 0.0, 0.0, 0.0];
        let v1 = engine
            .storage
            .vectors
            .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &v1_data, None, &arena)
            .unwrap();

        let v2_data = vec![0.0f64, 1.0, 0.0, 0.0];
        let _v2 = engine
            .storage
            .vectors
            .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &v2_data, None, &arena)
            .unwrap();

        engine
            .storage
            .vectors
            .delete(&mut txn, v1.id, &arena)
            .unwrap();

        txn.commit().unwrap();

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_request(),
        };

        let result = diagnostics_inner(input);
        assert!(result.is_ok());

        let response = result.unwrap();
        let body_str = String::from_utf8(response.body).unwrap();
        assert!(body_str.contains("\"total\":2"));
        assert!(body_str.contains("\"active\":1"));
        assert!(body_str.contains("\"soft_deleted\":1"));
        assert!(body_str.contains("\"entry_point_present\":true"));
        Ok(())
    }

    fn make_integrity_request() -> Request {
        Request {
            name: "hnsw_integrity".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
            pre_computed_embedding: None,
        }
    }

    #[test]
    fn test_hnsw_integrity_empty_db_is_symmetric() {
        let (engine, _temp_dir) = setup_test_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_integrity_request(),
        };

        let result = hnsw_integrity_inner(input);
        assert!(result.is_ok(), "hnsw_integrity on empty db should succeed: {result:?}");

        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(body.contains("\"symmetric\":true"), "empty db should be symmetric, got: {body}");
        assert!(body.contains("\"asymmetric_edges\":0"), "got: {body}");
        assert!(body.contains("\"total_edges\":0"), "got: {body}");
    }

    #[test]
    fn test_hnsw_integrity_after_inserts_is_symmetric() -> Result<(), Box<dyn std::error::Error>> {
        use crate::sparrow_engine::vector_core::HNSW;

        let (engine, _temp_dir) = setup_test_engine();
        let arena = bumpalo::Bump::new();
        let mut txn = engine.storage.graph_env.write_txn().unwrap();

        for i in 1..=10i64 {
            let data = vec![i as f64, 0.0, 0.0, 0.0];
            engine.storage.vectors
                .insert::<fn(&_, &_) -> bool>(&mut txn, "default", &data, None, &arena)
                .unwrap();
        }
        txn.commit().unwrap();

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_integrity_request(),
        };

        let result = hnsw_integrity_inner(input)?;
        let body = String::from_utf8(result.body).unwrap();
        assert!(body.contains("\"symmetric\":true"), "10 inserts should be symmetric, got: {body}");
        assert!(body.contains("\"asymmetric_edges\":0"), "got: {body}");
        Ok(())
    }
}
