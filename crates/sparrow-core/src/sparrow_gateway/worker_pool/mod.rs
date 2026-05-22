use crate::sparrow_engine::{traversal_core::SparrowGraphEngine, types::GraphError};
use crate::sparrow_gateway::{
    embedding_providers::{EmbeddingModel, get_embedding_model},
    gateway::CoreSetter,
    mcp::mcp::MCPToolInput,
    router::router::{ContChan, ContMsg, HandlerInput, SparrowRouter},
};
use crate::protocol::{
    SparrowError, Request,
    request::{ReqMsg, RequestType, RetChan},
    response::Response,
};
use flume::{Receiver, Sender};
use std::iter;
use std::sync::Arc;
use std::thread::JoinHandle;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tracing::{error, trace};

/// A Thread Pool of workers to execute Database operations
pub struct WorkerPool {
    tx: Sender<ReqMsg>,
    write_tx: Sender<ReqMsg>,
    router: Arc<SparrowRouter>,
    _workers: Vec<Worker>,
    _writer_worker: Worker,
}

impl WorkerPool {
    pub fn new(
        workers_core_setter: Arc<CoreSetter>,
        graph_access: Arc<SparrowGraphEngine>,
        router: Arc<SparrowRouter>,
        io_rt: Arc<Runtime>,
    ) -> WorkerPool {
        let (req_tx, req_rx) = flume::bounded::<ReqMsg>(1000);
        let (cont_tx, cont_rx) = flume::bounded::<ContMsg>(1000);

        // Dedicated channel for write operations - single writer thread
        let (write_tx, write_rx) = flume::bounded::<ReqMsg>(1000);

        let num_workers = workers_core_setter.num_threads();
        if num_workers < 2 {
            panic!("The number of workers must be at least 2 for parity to act as a select.");
        }
        if !num_workers.is_multiple_of(2) {
            println!("Expected an even number of workers, got {num_workers}");
            panic!("The number of workers should be a multiple of 2 for fairness.");
        }

        let workers = iter::repeat_n(workers_core_setter, num_workers)
            .enumerate()
            .map(|(i, setter)| {
                Worker::start(
                    req_rx.clone(),
                    setter,
                    Arc::clone(&graph_access),
                    Arc::clone(&router),
                    Arc::clone(&io_rt),
                    (cont_tx.clone(), cont_rx.clone()),
                    i % 2 == 0,
                )
            })
            .collect();

        // Create the dedicated writer worker (no core pinning needed for single thread)
        let writer_worker = Worker::start_writer(
            write_rx,
            Arc::clone(&graph_access),
            Arc::clone(&router),
            Arc::clone(&io_rt),
        );

        WorkerPool {
            tx: req_tx,
            write_tx,
            router,
            _workers: workers,
            _writer_worker: writer_worker,
        }
    }

    /// Check if a route name is a write operation.
    pub fn is_write_route(&self, name: &str) -> bool {
        self.router.is_write_route(name)
    }

    /// Process a request on the Worker Pool
    /// Write operations are routed to a dedicated writer thread to ensure proper LMDB locking
    pub async fn process(&self, mut req: Request) -> Result<Response, SparrowError> {
        let (ret_tx, ret_rx) = oneshot::channel();
        let req_name = req.name.clone();

        // For search_vector_text: pre-compute the embedding here in the async context
        // so the sync worker thread never needs to call block_on for an embedding API call,
        // which would deadlock the Tokio runtime.
        //
        // Note: this pre-computation covers the MCP `search_vector_text` route.
        // HQL-compiled queries that use vector similarity (`embed_async!` macro in generated code)
        // run on the Query dispatch path. Those paths use `spawn_blocking` or a dedicated
        // runtime handle — verify if they're affected before adding more embed routes here.
        if req.name == "search_vector_text" {
            #[derive(serde::Deserialize)]
            struct QueryBody {
                data: QueryData,
            }
            #[derive(serde::Deserialize)]
            struct QueryData {
                query: String,
            }

            match sonic_rs::from_slice::<QueryBody>(&req.body) {
                Ok(parsed) => {
                    match get_embedding_model(None, None, None) {
                        Ok(model) => {
                            match model.fetch_embedding_async(&parsed.data.query).await {
                                Ok(embedding) => {
                                    req.pre_computed_embedding = Some(embedding);
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "[VECTOR_SEARCH] Failed to pre-compute embedding: {:?}",
                                        e
                                    );
                                    return Err(SparrowError::Graph(e));
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "[VECTOR_SEARCH] Failed to get embedding model: {:?}",
                                e
                            );
                            return Err(SparrowError::Graph(e));
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "[VECTOR_SEARCH] Failed to parse body for embedding pre-computation: {:?}",
                        e
                    );
                    return Err(SparrowError::Graph(GraphError::from(e)));
                }
            }
        }

        // Route to dedicated writer thread or reader worker pool
        let channel = if self.router.is_write_route(&req.name) {
            &self.write_tx
        } else {
            &self.tx
        };

        channel.send_async((req, ret_tx)).await.map_err(|_| {
            error!("WorkerPool channel closed for request '{req_name}'");
            SparrowError::Graph(GraphError::New("Server is shutting down".into()))
        })?;

        // Handle the case where the worker might have dropped the sender
        // (e.g., worker thread panicked or client disconnected)
        ret_rx.await.unwrap_or_else(|_| {
            error!("Worker dropped sender without reply for request '{req_name}'");
            Err(SparrowError::Graph(GraphError::New(
                "Internal server error: worker failed to respond".into(),
            )))
        })
    }
}

struct Worker {
    _handle: JoinHandle<()>,
}

impl Worker {
    pub fn start(
        rx: Receiver<ReqMsg>,
        core_setter: Arc<CoreSetter>,
        graph_access: Arc<SparrowGraphEngine>,
        router: Arc<SparrowRouter>,
        io_rt: Arc<Runtime>,
        (cont_tx, cont_rx): (ContChan, Receiver<ContMsg>),
        parity: bool,
    ) -> Worker {
        let handle = std::thread::spawn(move || {
            core_setter.set_current();

            // Initialize thread-local metrics buffer
            sparrow_metrics::init_thread_local();

            // Set thread local context, so we can access the io runtime
            let _io_guard = io_rt.enter();

            // To avoid a select, we try_recv on one channel and then wait on the other.
            // Since we have multiple workers, we use parity to decide which order around,
            // meaning if there's at least 2 worker threads its a fair select.
            match parity {
                true => {
                    loop {
                        // cont_rx.try_recv() then rx.recv()

                        match cont_rx.try_recv() {
                            Ok((ret_chan, cfn)) => {
                                let result = cfn().map_err(Into::into);
                                if ret_chan.send(result).is_err() {
                                    trace!(
                                        "Client disconnected before continuation response could be sent"
                                    );
                                }
                            }
                            Err(flume::TryRecvError::Disconnected) => {
                                error!("Continuation Channel was dropped");
                                break;
                            }
                            Err(flume::TryRecvError::Empty) => {}
                        }

                        match rx.recv() {
                            Ok((req, ret_chan)) => request_mapper(
                                req,
                                ret_chan,
                                graph_access.clone(),
                                &router,
                                &io_rt,
                                &cont_tx,
                            ),
                            Err(flume::RecvError::Disconnected) => {
                                error!("Request Channel was dropped");
                                break;
                            }
                        }
                    }
                }
                false => {
                    loop {
                        // rx.try_recv() then cont_rx.recv()

                        match rx.try_recv() {
                            Ok((req, ret_chan)) => request_mapper(
                                req,
                                ret_chan,
                                graph_access.clone(),
                                &router,
                                &io_rt,
                                &cont_tx,
                            ),
                            Err(flume::TryRecvError::Disconnected) => {
                                error!("Request Channel was dropped");
                                break;
                            }
                            Err(flume::TryRecvError::Empty) => {}
                        }

                        match cont_rx.recv() {
                            Ok((ret_chan, cfn)) => {
                                let result = cfn().map_err(Into::into);
                                if ret_chan.send(result).is_err() {
                                    trace!(
                                        "Client disconnected before continuation response could be sent"
                                    );
                                }
                            }
                            Err(flume::RecvError::Disconnected) => {
                                error!("Continuation Channel was dropped");
                                break;
                            }
                        }
                    }
                }
            }
        });
        Worker { _handle: handle }
    }

    /// Start a dedicated writer worker thread
    /// This thread handles all write operations to ensure proper LMDB locking
    /// Note: No core pinning for the writer - let the OS scheduler handle it
    pub fn start_writer(
        rx: Receiver<ReqMsg>,
        graph_access: Arc<SparrowGraphEngine>,
        router: Arc<SparrowRouter>,
        io_rt: Arc<Runtime>,
    ) -> Worker {
        let handle = std::thread::spawn(move || {
            // Initialize thread-local metrics buffer
            sparrow_metrics::init_thread_local();

            // Set thread local context, so we can access the io runtime
            let _io_guard = io_rt.enter();

            // Single-threaded writer: process one request at a time, waiting for
            // any continuations to complete before moving to the next request.
            loop {
                match rx.recv() {
                    Ok((req, ret_chan)) => {
                        // Create a per-request continuation channel
                        let (cont_tx, cont_rx) = flume::bounded::<ContMsg>(1);

                        // Process the request
                        request_mapper(
                            req,
                            ret_chan,
                            graph_access.clone(),
                            &router,
                            &io_rt,
                            &cont_tx,
                        );

                        // Drop our sender so the channel disconnects when the async future
                        // (which holds a clone) completes.
                        drop(cont_tx);

                        // Poll continuation channel until sender is dropped.
                        while let Ok((ret_chan, cfn)) = cont_rx.recv() {
                            let result = cfn().map_err(Into::into);
                            if ret_chan.send(result).is_err() {
                                trace!(
                                    "Client disconnected before continuation response could be sent"
                                );
                            }
                        }
                    }
                    Err(_) => {
                        trace!("Writer request channel was dropped, shutting down");
                        break;
                    }
                }
            }
        });
        Worker { _handle: handle }
    }
}

fn request_mapper(
    mut request: Request,
    ret_chan: RetChan,
    graph_access: Arc<SparrowGraphEngine>,
    router: &SparrowRouter,
    io_rt: &Runtime,
    cont_tx: &ContChan,
) {
    let req_name = request.name.clone();
    let req_type = request.req_type;

    let res = match request.req_type {
        RequestType::Query => {
            if let Some(handler) = router.routes.get(&request.name) {
                let input = HandlerInput {
                    request,
                    graph: graph_access,
                };

                match handler(input) {
                    Err(GraphError::IoNeeded(cont_closure)) => {
                        let fut = cont_closure.0(cont_tx.clone(), ret_chan);
                        io_rt.spawn(fut);
                        return;
                    }
                    res => Some(res.map_err(Into::into)),
                }
            } else {
                None
            }
        }
        RequestType::MCP => {
            if let Some(mcp_handler) = router.mcp_routes.get(&request.name) {
                let embedding = request.pre_computed_embedding.take();
                let mut mcp_input = MCPToolInput {
                    request,
                    mcp_backend: Arc::clone(
                        graph_access
                            .mcp_backend
                            .as_ref()
                            .expect("MCP backend not found"),
                    ),
                    mcp_connections: Arc::clone(
                        graph_access
                            .mcp_connections
                            .as_ref()
                            .expect("MCP connections not found"),
                    ),
                    schema: graph_access.storage.storage_config.schema.clone(),
                    embedding,
                };
                Some(mcp_handler(&mut mcp_input).map_err(Into::into))
            } else {
                None
            }
        }
    };

    let res = res.unwrap_or(Err(SparrowError::NotFound {
        ty: req_type,
        name: req_name.clone(),
    }));

    // Client may have disconnected before we could send the response.
    // This is normal behavior - just log at trace level and continue.
    if ret_chan.send(res).is_err() {
        trace!(
            "Client disconnected before response could be sent for request '{}'",
            req_name
        );
    }
}
