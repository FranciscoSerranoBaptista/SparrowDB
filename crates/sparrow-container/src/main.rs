use sparrow_db::sparrow_engine::{
    storage_core::version_info::{
        ItemInfo, Transition, TransitionFn, TransitionSubmission, VersionInfo,
    },
    traversal_core::{SparrowGraphEngine, SparrowGraphEngineOpts},
};
use sparrow_db::sparrow_gateway::mcp::mcp::{MCPHandlerFn, MCPHandlerSubmission};
use sparrow_db::sparrow_gateway::{
    gateway::{GatewayOpts, SparrowGateway},
    router::router::{HandlerFn, HandlerSubmission},
};
use std::{collections::HashMap, sync::Arc};
use tracing::info;
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

mod queries;

fn main() {
    let env_res = dotenvy::dotenv();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::filter::filter_fn(
                |metadata| {
                    let target = metadata.target();
                    !target.starts_with("axum")
                        && !target.starts_with("hyper")
                        && !target.starts_with("tower")
                        && !target.starts_with("h2")
                        && !target.starts_with("reqwest")
                },
            )),
        )
        .init();

    match env_res {
        Ok(_) => info!("Loaded .env file"),
        Err(e) => info!(?e, "Didn't load .env file"),
    }

    let config = queries::config().unwrap_or_default();

    let path = match std::env::var("SPARROW_DATA_DIR") {
        Ok(val) => std::path::PathBuf::from(val).join("user"),
        Err(_) => {
            println!("SPARROW_DATA_DIR not set, using default");
            let home = dirs::home_dir().expect("Could not retrieve home directory");
            home.join(".sparrow/user")
        }
    };

    let port = match std::env::var("SPARROW_PORT") {
        Ok(val) => val
            .parse::<u16>()
            .expect("SPARROW_PORT must be a valid port number"),
        Err(_) => 6969,
    };

    println!("Running with the following setup:");
    println!("\tconfig: {config:#?}");
    println!("\tpath: {}", path.display());
    println!("\tport: {port}");
    if matches!(std::env::var("SPARROW_SKIP_BM25_ON_WRITE").as_deref(), Ok("true") | Ok("1")) {
        println!(
            "\tSPARROW_SKIP_BM25_ON_WRITE=true — BM25 index updates DISABLED during writes. \
             Run POST /rebuild_bm25_index after bulk import to rebuild the index."
        );
    }

    let transition_fns: HashMap<&'static str, ItemInfo> =
        inventory::iter::<TransitionSubmission>.into_iter().fold(
            HashMap::new(),
            |mut acc,
             TransitionSubmission(Transition {
                 item_label,
                 func,
                 from_version,
                 to_version,
                 ..
             })| {
                acc.entry(item_label)
                    .and_modify(|item_info: &mut ItemInfo| {
                        item_info.latest = item_info.latest.max(*to_version);

                        // asserts for versions
                        assert!(
                            *from_version < *to_version,
                            "from_version must be less than to_version"
                        );
                        assert!(*from_version > 0, "from_version must be greater than 0");
                        assert!(*to_version > 0, "to_version must be greater than 0");
                        assert!(
                            *to_version - *from_version == 1,
                            "to_version must be exactly 1 greater than from_version"
                        );

                        item_info.transition_fns.push(TransitionFn {
                            from_version: *from_version,
                            to_version: *to_version,
                            func: *func,
                        });
                        item_info.transition_fns.sort_by_key(|f| f.from_version);
                    });
                acc
            },
        );

    let path_str = path.to_str().expect("Could not convert path to string");
    let hql_schema_raw = config.hql_schema_raw.clone();
    let opts = SparrowGraphEngineOpts {
        path: path_str.to_string(),
        config,
        version_info: VersionInfo(transition_fns),
    };

    let graph = Arc::new(
        SparrowGraphEngine::new(opts.clone())
            .unwrap_or_else(|e| panic!("Failed to create graph engine: {e}")),
    );

    // generates routes from handler proc macro
    let submissions: Vec<_> = inventory::iter::<HandlerSubmission>.into_iter().collect();
    println!("Found {} route submissions", submissions.len());

    let (mut query_routes, mut write_routes): (
        HashMap<String, HandlerFn>,
        std::collections::HashSet<String>,
    ) = inventory::iter::<HandlerSubmission>.into_iter().fold(
        (HashMap::new(), std::collections::HashSet::new()),
        |(mut routes, mut writes), submission| {
            println!(
                "Processing POST submission for handler: {} (is_write: {})",
                submission.0.name, submission.0.is_write
            );
            let handler = &submission.0;
            let func: HandlerFn = Arc::new(handler.func);
            routes.insert(handler.name.to_string(), func);
            if handler.is_write {
                writes.insert(handler.name.to_string());
            }
            (routes, writes)
        },
    );

    // Runtime HQL eval — always on when studio is compiled in; opt-in via env otherwise
    #[cfg(feature = "studio")]
    let hql_eval_enabled = true;
    #[cfg(not(feature = "studio"))]
    let hql_eval_enabled = std::env::var("SPARROW_RUNTIME_HQL").as_deref() == Ok("true");

    if hql_eval_enabled {
        use sparrow_db::sparrow_gateway::runtime_eval::handler as runtime_handler;
        let rt_handler: HandlerFn = Arc::new(move |input| {
            runtime_handler::handle(input, hql_schema_raw.clone())
        });
        query_routes.insert("__hql_runtime_eval".to_string(), rt_handler);
        write_routes.insert("__hql_runtime_eval".to_string());
        println!("Runtime HQL eval enabled at POST /__hql_runtime_eval");
    }

    let mcp_routes = inventory::iter::<MCPHandlerSubmission>
        .into_iter()
        .map(|submission| {
            println!("Processing submission for handler: {}", submission.0.name);
            let handler = &submission.0;
            let func: MCPHandlerFn = Arc::new(handler.func);
            (handler.name.to_string(), func)
        })
        .collect::<HashMap<String, MCPHandlerFn>>();

    println!("Routes: {:?}", query_routes.keys());
    println!("Write routes: {:?}", write_routes);
    let gateway = SparrowGateway::new(
        &format!("0.0.0.0:{port}"),
        graph,
        GatewayOpts::DEFAULT_WORKERS_PER_CORE,
        Some(query_routes),
        Some(mcp_routes),
        Some(write_routes),
        Some(opts),
    );

    gateway.run().expect("Failed to run gateway")
}
