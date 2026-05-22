use std::collections::HashSet;
use std::sync::atomic::{self, AtomicUsize};
use std::time::Instant;
use std::{collections::HashMap, sync::Arc};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use core_affinity::CoreId;
use tracing::{info, trace, warn};

use super::router::router::{HandlerFn, SparrowRouter};
use crate::sparrow_gateway::v1_compat::v1_query_axum_handler;
#[cfg(feature = "lmdb")]
use crate::sparrow_gateway::auth::TokenStore;
#[cfg(feature = "dev-instance")]
use crate::sparrow_gateway::builtin::all_nodes_and_edges::nodes_edges_handler;
#[cfg(feature = "dev-instance")]
use crate::sparrow_gateway::builtin::node_by_id::node_details_handler;
#[cfg(feature = "dev-instance")]
use crate::sparrow_gateway::builtin::node_connections::node_connections_handler;
#[cfg(feature = "dev-instance")]
use crate::sparrow_gateway::builtin::nodes_by_label::nodes_by_label_handler;
use crate::sparrow_gateway::introspect_schema::introspect_schema_handler;
use crate::sparrow_gateway::worker_pool::WorkerPool;
use crate::protocol;
use crate::protocol::SparrowError;
use crate::{
    sparrow_engine::traversal_core::{SparrowGraphEngine, SparrowGraphEngineOpts},
    sparrow_gateway::mcp::mcp::MCPHandlerFn,
};

pub struct GatewayOpts {}

impl GatewayOpts {
    pub const DEFAULT_WORKERS_PER_CORE: usize = 8;
}

pub struct SparrowGateway {
    pub(crate) address: String,
    pub(crate) workers_per_core: usize,
    pub(crate) graph_access: Arc<SparrowGraphEngine>,
    pub(crate) router: Arc<SparrowRouter>,
    pub(crate) opts: Option<SparrowGraphEngineOpts>,
    pub(crate) cluster_id: Option<String>,
    #[cfg(feature = "lmdb")]
    pub(crate) token_store: Arc<TokenStore>,
}

impl SparrowGateway {
    pub fn new(
        address: &str,
        graph_access: Arc<SparrowGraphEngine>,
        workers_per_core: usize,
        routes: Option<HashMap<String, HandlerFn>>,
        mcp_routes: Option<HashMap<String, MCPHandlerFn>>,
        write_routes: Option<HashSet<String>>,
        opts: Option<SparrowGraphEngineOpts>,
    ) -> SparrowGateway {
        let router = Arc::new(SparrowRouter::new(routes, mcp_routes, write_routes));
        let cluster_id = std::env::var("SPARROW_CLUSTER_ID").ok();
        #[cfg(feature = "lmdb")]
        let token_store = {
            let auth_path = opts.as_ref()
                .and_then(|o| {
                    let parent = std::path::Path::new(&o.path).parent()?;
                    // `parent()` returns Some("") for bare filenames — treat as absent.
                    if parent.as_os_str().is_empty() {
                        None
                    } else {
                        Some(parent.join("auth"))
                    }
                })
                .unwrap_or_else(|| {
                    // Tests: unique temp path per gateway instance avoids conflicts
                    let rnd: u64 = rand::random();
                    std::path::PathBuf::from(format!("/tmp/sparrow_auth_{rnd:x}"))
                });

            let store = TokenStore::open(
                auth_path.to_str().expect("auth path is valid UTF-8"),
            ).expect("failed to open token store");

            // Seed SPARROW_API_KEY as admin token for backward compatibility
            if let Ok(legacy_key) = std::env::var("SPARROW_API_KEY") {
                if !legacy_key.is_empty() {
                    store.seed_legacy(&legacy_key);
                }
            }

            Arc::new(store)
        };
        SparrowGateway {
            address: address.to_string(),
            graph_access,
            router,
            workers_per_core,
            opts,
            cluster_id,
            #[cfg(feature = "lmdb")]
            token_store,
        }
    }

    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        trace!("Starting Sparrow Gateway");

        let all_core_ids = core_affinity::get_core_ids().expect("unable to get core IDs");

        let all_core_ids = match std::env::var("SPARROW_CORES_OVERRIDE") {
            Ok(val) => {
                let override_count: usize = val
                    .parse()
                    .expect("SPARROW_CORES_OVERRIDE must be a valid number");
                if override_count > all_core_ids.len() {
                    warn!(
                        "SPARROW_CORES_OVERRIDE ({}) exceeds available cores ({}), using all cores",
                        override_count,
                        all_core_ids.len()
                    );
                    all_core_ids
                } else {
                    all_core_ids.into_iter().take(override_count).collect()
                }
            }
            Err(_) => all_core_ids,
        };

        info!(
            "Worker pool initialized: {} cores, {} worker threads, 1 writer thread",
            all_core_ids.len(),
            all_core_ids.len() * self.workers_per_core
        );

        let tokio_core_ids = all_core_ids.clone();
        let tokio_core_setter = Arc::new(CoreSetter::new(tokio_core_ids, 1));

        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(tokio_core_setter.num_threads())
                .on_thread_unpark(move || Arc::clone(&tokio_core_setter).set_current_once())
                .enable_all()
                .build()?,
        );

        let worker_core_ids = all_core_ids.clone();
        let worker_core_setter = Arc::new(CoreSetter::new(worker_core_ids, self.workers_per_core));

        let worker_pool = WorkerPool::new(
            worker_core_setter,
            Arc::clone(&self.graph_access),
            Arc::clone(&self.router),
            Arc::clone(&rt),
        );

        let mut axum_app = axum::Router::new();

        // /v1/query MUST be registered before /{*path}: the wildcard handler rejects
        // any path whose name component contains '/', which "v1/query" does.
        axum_app = axum_app
            .route("/v1/query", post(v1_query_axum_handler))
            .route("/{*path}", post(post_handler))
            .route("/introspect", get(introspect_schema_handler));

        #[cfg(feature = "dev-instance")]
        {
            axum_app = axum_app
                .route("/nodes-edges", get(nodes_edges_handler))
                .route("/nodes-by-label", get(nodes_by_label_handler))
                .route("/node-connections", get(node_connections_handler))
                .route("/node-details", get(node_details_handler));
        }

        #[cfg(feature = "lmdb")]
        {
            use crate::sparrow_gateway::builtin::token_mgmt::{
                create_token_handler, list_tokens_handler, revoke_token_handler,
            };
            use axum::routing::delete;
            axum_app = axum_app
                .route("/tokens", get(list_tokens_handler).post(create_token_handler))
                .route("/tokens/{id}", delete(revoke_token_handler));
        }

        #[cfg(feature = "studio")]
        {
            axum_app = axum_app.merge(sparrow_studio::studio_router());
        }

        let axum_app = axum_app.with_state(Arc::new(AppState {
            worker_pool,
            schema_json: self.opts.and_then(|o| o.config.schema.map(Bytes::from)),
            cluster_id: self.cluster_id,
            #[cfg(feature = "lmdb")]
            token_store: Arc::clone(&self.token_store),
        }));

        rt.block_on(async move {
            // Initialize metrics system
            sparrow_metrics::init_metrics_system();

            let listener = tokio::net::TcpListener::bind(self.address)
                .await
                .expect("Failed to bind listener");
            info!("Listener has been bound, starting server");
            axum::serve(listener, axum_app)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .expect("Failed to serve");

            // Shutdown metrics system to flush all pending events
            info!("Shutting down metrics system...");
            let shutdown_result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                sparrow_metrics::shutdown_metrics_system(),
            )
            .await;

            match shutdown_result {
                Ok(_) => info!("Metrics system shutdown complete"),
                Err(_) => warn!("Metrics system shutdown timed out after 5 seconds"),
            }
        });

        Ok(())
    }
}

async fn shutdown_signal() {
    // Respond to either Ctrl-C (SIGINT) or SIGTERM (e.g. `kill` or systemd stop)
    #[cfg(unix)]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl-C, starting graceful shutdown…");
            }
            _ = sigterm() => {
                info!("Received SIGTERM, starting graceful shutdown…");
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        info!("Received Ctrl-C, starting graceful shutdown…");
    }
}

#[cfg(unix)]
async fn sigterm() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    term.recv().await;
}

async fn post_handler(
    State(state): State<Arc<AppState>>,
    req: protocol::request::Request,
) -> axum::http::Response<Body> {
    let start_time = Instant::now();
    #[cfg(feature = "lmdb")]
    {
        use crate::sparrow_gateway::auth::TokenError;
        if state.token_store.is_auth_required() {
            let raw_key = req.api_key.as_deref().unwrap_or("");
            match state.token_store.verify(raw_key) {
                Ok(record) => {
                    if state.worker_pool.is_write_route(&req.name) && !record.role.can_write() {
                        return SparrowError::Forbidden.into_response();
                    }
                }
                Err(TokenError::InvalidKey) | Err(TokenError::Unauthorized) => {
                    sparrow_metrics::log_event(
                        sparrow_metrics::events::EventType::InvalidApiKey,
                        sparrow_metrics::events::InvalidApiKeyEvent {
                            cluster_id: state.cluster_id.clone(),
                            time_taken_usec: start_time.elapsed().as_micros() as u32,
                        },
                    );
                    return SparrowError::InvalidApiKey.into_response();
                }
                Err(_) => return SparrowError::InvalidApiKey.into_response(),
            }
        }
    }
    let input_body = if *sparrow_metrics::METRICS_ENABLED {
        Some(req.body.clone())
    } else {
        None
    };
    let query_name = req.name.clone();
    let res = state.worker_pool.process(req).await;

    match res {
        Ok(r) => {
            #[cfg(any(feature = "dev-instance", feature = "production"))]
            {
                let resp_str = String::from_utf8_lossy(&r.body);
                info!(query = %query_name, response = %resp_str, "Response");
            }
            if !*sparrow_metrics::METRICS_ENABLED {
                return r.into_response();
            }
            sparrow_metrics::log_event(
                sparrow_metrics::events::EventType::QuerySuccess,
                sparrow_metrics::events::QuerySuccessEvent {
                    cluster_id: state.cluster_id.clone(),
                    query_name,
                    time_taken_usec: start_time.elapsed().as_micros() as u32,
                },
            );
            r.into_response()
        }
        Err(e) => {
            info!(query = %query_name, error = ?e, "Error response");
            if !*sparrow_metrics::METRICS_ENABLED {
                return e.into_response();
            }
            sparrow_metrics::log_event(
                sparrow_metrics::events::EventType::QueryError,
                sparrow_metrics::events::QueryErrorEvent {
                    cluster_id: state.cluster_id.clone(),
                    query_name,
                    input_json: input_body
                        .as_ref()
                        .and_then(|body| std::str::from_utf8(body.as_ref()).ok())
                        .map(str::to_owned),
                    output_json: sonic_rs::to_string(&e).ok(),
                    time_taken_usec: start_time.elapsed().as_micros() as u32,
                },
            );
            e.into_response()
        }
    }
}

pub struct AppState {
    pub worker_pool: WorkerPool,
    pub schema_json: Option<Bytes>,
    pub cluster_id: Option<String>,
    #[cfg(feature = "lmdb")]
    pub token_store: Arc<TokenStore>,
}

pub struct CoreSetter {
    pub(crate) cores: Vec<CoreId>,
    pub(crate) threads_per_core: usize,
    pub(crate) incrementing_index: AtomicUsize,
}

impl CoreSetter {
    pub fn new(cores: Vec<CoreId>, threads_per_core: usize) -> Self {
        Self {
            cores,
            threads_per_core,
            incrementing_index: AtomicUsize::new(0),
        }
    }

    pub fn num_threads(&self) -> usize {
        self.cores.len() * self.threads_per_core
    }

    pub fn set_current(self: Arc<Self>) {
        let curr_idx = self
            .incrementing_index
            .fetch_add(1, atomic::Ordering::SeqCst);

        let core_index = curr_idx / self.threads_per_core;
        match self.cores.get(core_index) {
            Some(c) => {
                core_affinity::set_for_current(*c);
            }
            None => warn!(
                "CoreSetter::set_current called more times than cores.len() * threads_per_core. Core affinity not set"
            ),
        };
    }

    pub fn set_current_once(self: Arc<Self>) {
        use std::sync::OnceLock;

        thread_local! {
            static CORE_SET: OnceLock<()> = const { OnceLock::new() };
        }

        CORE_SET.with(|flag| {
            flag.get_or_init(move || self.set_current());
        });
    }
}
