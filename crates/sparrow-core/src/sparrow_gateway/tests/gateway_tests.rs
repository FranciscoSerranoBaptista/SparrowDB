use crate::sparrow_engine::traversal_core::{SparrowGraphEngine, SparrowGraphEngineOpts};
use crate::sparrow_gateway::gateway::{AppState, CoreSetter, GatewayOpts, SparrowGateway};
use crate::sparrow_gateway::router::router::SparrowRouter;
use crate::sparrow_gateway::worker_pool::WorkerPool;
use axum::body::Bytes;
use core_affinity::CoreId;
use std::sync::atomic;
use std::{collections::HashMap, sync::Arc};
#[cfg(feature = "lmdb")]
use crate::sparrow_gateway::auth::TokenStore;

use crate::sparrow_engine::traversal_core::config::Config;
use tempfile::TempDir;

fn create_test_graph() -> (Arc<SparrowGraphEngine>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let opts = SparrowGraphEngineOpts {
        path: temp_dir.path().to_str().unwrap().to_string(),
        config: Config::default(),
        version_info: Default::default(),
        skip_bm25_on_write: None,
    };
    let graph = Arc::new(SparrowGraphEngine::new(opts).unwrap());
    (graph, temp_dir)
}

// ============================================================================
// SparrowGateway Tests
// ============================================================================

#[test]
fn test_gateway_new_basic() {
    let (graph, _temp_dir) = create_test_graph();
    let gateway = SparrowGateway::new("127.0.0.1:8080", graph, 8, None, None, None, None);

    assert_eq!(gateway.address, "127.0.0.1:8080");
    assert_eq!(gateway.workers_per_core, 8);
    assert!(gateway.opts.is_none());
}

#[test]
fn test_gateway_new_with_routes() {
    let (graph, _temp_dir) = create_test_graph();
    let routes = HashMap::new();
    let gateway = SparrowGateway::new("127.0.0.1:8080", graph, 8, Some(routes), None, None, None);

    assert_eq!(gateway.address, "127.0.0.1:8080");
    assert!(gateway.router.routes.is_empty());
}

#[test]
fn test_gateway_new_with_mcp_routes() {
    let (graph, _temp_dir) = create_test_graph();
    let mcp_routes = HashMap::new();
    let gateway = SparrowGateway::new(
        "127.0.0.1:8080",
        graph,
        8,
        None,
        Some(mcp_routes),
        None,
        None,
    );

    assert_eq!(gateway.address, "127.0.0.1:8080");
    assert!(gateway.router.mcp_routes.is_empty());
}

#[test]
fn test_gateway_new_with_opts() {
    let (graph, temp_dir) = create_test_graph();
    let opts = SparrowGraphEngineOpts {
        path: temp_dir.path().to_str().unwrap().to_string(),
        config: Config::default(),
        version_info: Default::default(),
        skip_bm25_on_write: None,
    };
    let gateway = SparrowGateway::new("127.0.0.1:8080", graph, 8, None, None, None, Some(opts));

    assert!(gateway.opts.is_some());
}

#[test]
fn test_gateway_new_with_cluster_id() {
    unsafe {
        std::env::set_var("SPARROW_CLUSTER_ID", "test-cluster-123");
    }
    let (graph, _temp_dir) = create_test_graph();
    let gateway = SparrowGateway::new("127.0.0.1:8080", graph, 8, None, None, None, None);

    assert!(gateway.cluster_id.is_some());
    assert_eq!(gateway.cluster_id.unwrap(), "test-cluster-123");
    unsafe {
        std::env::remove_var("SPARROW_CLUSTER_ID");
    }
}

#[test]
fn test_gateway_fields() {
    let (graph, _temp_dir) = create_test_graph();
    let gateway = SparrowGateway::new("0.0.0.0:3000", graph, 10, None, None, None, None);

    assert_eq!(gateway.address, "0.0.0.0:3000");
    assert_eq!(gateway.workers_per_core, 10);
}

#[test]
fn test_gateway_address_format() {
    let (graph, _temp_dir) = create_test_graph();
    let gateway = SparrowGateway::new("localhost:8080", graph.clone(), 1, None, None, None, None);
    assert_eq!(gateway.address, "localhost:8080");

    let gateway2 = SparrowGateway::new("0.0.0.0:80", graph, 1, None, None, None, None);
    assert_eq!(gateway2.address, "0.0.0.0:80");
}

#[test]
fn test_gateway_workers_per_core() {
    let (graph, _temp_dir) = create_test_graph();

    let gateway1 = SparrowGateway::new("127.0.0.1:8080", graph.clone(), 1, None, None, None, None);
    assert_eq!(gateway1.workers_per_core, 1);

    let gateway2 = SparrowGateway::new("127.0.0.1:8080", graph.clone(), 10, None, None, None, None);
    assert_eq!(gateway2.workers_per_core, 10);

    let gateway3 = SparrowGateway::new(
        "127.0.0.1:8080",
        graph,
        GatewayOpts::DEFAULT_WORKERS_PER_CORE,
        None,
        None,
        None,
        None,
    );
    assert_eq!(gateway3.workers_per_core, 4);
}

// ============================================================================
// AppState Tests
// ============================================================================

#[test]
fn test_app_state_creation() {
    let (graph, _temp_dir) = create_test_graph();
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

    let state = AppState {
        worker_pool,
        schema_json: None,
        cluster_id: None,
        #[cfg(feature = "lmdb")]
        token_store: {
            let rnd: u64 = rand::random();
            Arc::new(TokenStore::open(&format!("/tmp/sparrow_auth_{rnd:x}")).unwrap())
        },
    };

    assert!(state.schema_json.is_none());
    assert!(state.cluster_id.is_none());
}

#[test]
fn test_app_state_with_schema() {
    let (graph, _temp_dir) = create_test_graph();
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

    let state = AppState {
        worker_pool,
        schema_json: Some(Bytes::from_static(br#"{"schema": "test"}"#)),
        cluster_id: None,
        #[cfg(feature = "lmdb")]
        token_store: {
            let rnd: u64 = rand::random();
            Arc::new(TokenStore::open(&format!("/tmp/sparrow_auth_{rnd:x}")).unwrap())
        },
    };

    assert!(state.schema_json.is_some());
    assert_eq!(
        state.schema_json.unwrap(),
        Bytes::from_static(br#"{"schema": "test"}"#)
    );
}

#[test]
fn test_app_state_with_cluster_id() {
    let (graph, _temp_dir) = create_test_graph();
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

    let state = AppState {
        worker_pool,
        schema_json: None,
        cluster_id: Some("cluster-456".to_string()),
        #[cfg(feature = "lmdb")]
        token_store: {
            let rnd: u64 = rand::random();
            Arc::new(TokenStore::open(&format!("/tmp/sparrow_auth_{rnd:x}")).unwrap())
        },
    };

    assert!(state.cluster_id.is_some());
    assert_eq!(state.cluster_id.unwrap(), "cluster-456");
}

// ============================================================================
// CoreSetter Tests
// ============================================================================

#[test]
fn test_core_setter_new() {
    let cores = vec![CoreId { id: 0 }, CoreId { id: 1 }];
    let setter = CoreSetter::new(cores.clone(), 8);

    assert_eq!(setter.cores.len(), 2);
    assert_eq!(setter.threads_per_core, 8);
}

#[test]
fn test_core_setter_num_threads_single_core() {
    let cores = vec![CoreId { id: 0 }];
    let setter = CoreSetter::new(cores, 1);

    assert_eq!(setter.num_threads(), 1);
}

#[test]
fn test_core_setter_num_threads_multiple_cores() {
    let cores = vec![CoreId { id: 0 }, CoreId { id: 1 }, CoreId { id: 2 }];
    let setter = CoreSetter::new(cores, 1);

    assert_eq!(setter.num_threads(), 3);
}

#[test]
fn test_core_setter_num_threads_multiple_threads_per_core() {
    let cores = vec![CoreId { id: 0 }, CoreId { id: 1 }];
    let setter = CoreSetter::new(cores, 8);

    assert_eq!(setter.num_threads(), 16);
}

#[test]
fn test_core_setter_num_threads_edge_cases() {
    // Zero cores
    let setter1 = CoreSetter::new(vec![], 8);
    assert_eq!(setter1.num_threads(), 0);

    // Zero threads per core
    let cores = vec![CoreId { id: 0 }, CoreId { id: 1 }];
    let setter2 = CoreSetter::new(cores, 0);
    assert_eq!(setter2.num_threads(), 0);
}

#[test]
fn test_core_setter_calculation() {
    let cores = vec![
        CoreId { id: 0 },
        CoreId { id: 1 },
        CoreId { id: 2 },
        CoreId { id: 3 },
    ];
    let setter = CoreSetter::new(cores, 8);

    assert_eq!(setter.num_threads(), 32);
}

#[test]
fn test_core_setter_empty_cores() {
    let setter = CoreSetter::new(vec![], 10);

    assert_eq!(setter.cores.len(), 0);
    assert_eq!(setter.num_threads(), 0);
}

#[test]
fn test_core_setter_single_thread() {
    let cores = vec![CoreId { id: 0 }, CoreId { id: 1 }];
    let setter = CoreSetter::new(cores, 1);

    assert_eq!(setter.threads_per_core, 1);
    assert_eq!(setter.num_threads(), 2);
}

#[test]
fn test_core_setter_many_threads() {
    let cores = vec![CoreId { id: 0 }];
    let setter = CoreSetter::new(cores, 100);

    assert_eq!(setter.num_threads(), 100);
}

#[test]
fn test_core_setter_num_threads_consistency() {
    let cores = vec![CoreId { id: 0 }, CoreId { id: 1 }];
    let setter = CoreSetter::new(cores, 8);

    assert_eq!(setter.num_threads(), 16);
}

#[test]
fn test_core_setter_threads_per_core_zero() {
    let cores = vec![CoreId { id: 0 }];
    let setter = CoreSetter::new(cores, 0);

    assert_eq!(setter.threads_per_core, 0);
    assert_eq!(setter.num_threads(), 0);
}

#[test]
fn test_core_setter_with_default_workers() {
    let cores = vec![CoreId { id: 0 }, CoreId { id: 1 }];
    let setter = CoreSetter::new(cores, GatewayOpts::DEFAULT_WORKERS_PER_CORE);

    assert_eq!(setter.threads_per_core, 4);
    assert_eq!(setter.num_threads(), 8);
}

#[test]
fn test_core_setter_index_initial_value() {
    let cores = vec![CoreId { id: 0 }];
    let setter = CoreSetter::new(cores, 1);

    assert_eq!(setter.incrementing_index.load(atomic::Ordering::SeqCst), 0);
}

#[test]
fn test_gateway_opts_default_workers_per_core() {
    assert_eq!(GatewayOpts::DEFAULT_WORKERS_PER_CORE, 4);
}

// ============================================================================
// TokenStore Tests
// ============================================================================

#[cfg(feature = "lmdb")]
#[test]
fn test_gateway_has_token_store() {
    let (graph, _temp_dir) = create_test_graph();
    let gateway = SparrowGateway::new("127.0.0.1:8080", graph, 8, None, None, None, None);
    // TokenStore must have been created — no tokens seeded in tests so auth is disabled
    assert!(!gateway.token_store.is_auth_required());
}

// ============================================================================
// TokenStore Auth Tests
// ============================================================================

#[cfg(feature = "lmdb")]
#[test]
fn test_verify_request_no_auth_required() {
    use crate::sparrow_gateway::auth::TokenStore;
    let dir = tempfile::tempdir().unwrap();
    let store = TokenStore::open(dir.path().to_str().unwrap()).unwrap();
    // No tokens → auth disabled → is_auth_required returns false
    assert!(!store.is_auth_required());
}

#[cfg(feature = "lmdb")]
#[test]
fn test_verify_request_auth_required_no_key() {
    use crate::sparrow_gateway::auth::{Role, TokenError, TokenStore};
    let dir = tempfile::tempdir().unwrap();
    let store = TokenStore::open(dir.path().to_str().unwrap()).unwrap();
    store.create("test", Role::ReadWrite).unwrap();
    // Auth is now required; empty key should fail
    let err = store.verify("").unwrap_err();
    assert!(matches!(err, TokenError::Unauthorized));
}

