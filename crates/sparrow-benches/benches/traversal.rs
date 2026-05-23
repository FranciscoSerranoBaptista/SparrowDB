//! Graph traversal benchmarks.
//!
//! Four groups:
//!   - `traversal/n_from_type/small`   — scan 100 nodes
//!   - `traversal/n_from_type/medium`  — scan 10 000 nodes
//!   - `traversal/out_node/small`      — single-hop out-edge, 100 node graph
//!   - `traversal/out_node/medium`     — single-hop out-edge, 10 000 node graph
//!
//! Setup (engine creation + seeding) happens ONCE per group — not per
//! iteration — so I/O is excluded from measured time.
//!
//! Run with:
//!   cargo bench -p sparrow-benches --bench traversal --features cpu

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use bumpalo::Bump;
use sparrow_db::sparrow_engine::traversal_core::{
    ops::{
        g::G,
        source::n_from_type::NFromTypeAdapter,
        out::out::OutAdapter,
    },
};
use sparrow_benches::{make_engine, seed_graph};

// ---------------------------------------------------------------------------
// n_from_type — how fast can we scan nodes by label?
// ---------------------------------------------------------------------------

fn bench_n_from_type_small(c: &mut Criterion) {
    let (engine, _tmp) = make_engine();
    seed_graph(engine.storage.as_ref(), 100);

    c.bench_function("traversal/n_from_type/small", |b| {
        let storage = engine.storage.as_ref();
        b.iter(|| {
            let arena = Bump::new();
            let rtxn = storage.graph_env.read_txn().unwrap();
            let count = G::new(storage, &rtxn, &arena)
                .n_from_type(black_box("person"))
                .take(200)
                .count();
            black_box(count)
        })
    });
}

fn bench_n_from_type_medium(c: &mut Criterion) {
    let (engine, _tmp) = make_engine();
    seed_graph(engine.storage.as_ref(), 10_000);

    c.bench_function("traversal/n_from_type/medium", |b| {
        let storage = engine.storage.as_ref();
        b.iter(|| {
            let arena = Bump::new();
            let rtxn = storage.graph_env.read_txn().unwrap();
            let count = G::new(storage, &rtxn, &arena)
                .n_from_type(black_box("person"))
                .take(20_000)
                .count();
            black_box(count)
        })
    });
}

// ---------------------------------------------------------------------------
// out_node — single-hop edge traversal
// ---------------------------------------------------------------------------

fn bench_out_node_small(c: &mut Criterion) {
    let (engine, _tmp) = make_engine();
    let ids = seed_graph(engine.storage.as_ref(), 100);
    // Use the first seeded node as traversal root
    let root_id = ids[0];

    c.bench_function("traversal/out_node/small", |b| {
        let storage = engine.storage.as_ref();
        b.iter(|| {
            let arena = Bump::new();
            let rtxn = storage.graph_env.read_txn().unwrap();
            let count = G::new(storage, &rtxn, &arena)
                .n_from_type("person")
                .out_node(black_box("knows"))
                .take(200)
                .count();
            black_box(count)
        })
    });

    let _ = root_id; // suppress unused warning
}

fn bench_out_node_medium(c: &mut Criterion) {
    let (engine, _tmp) = make_engine();
    let ids = seed_graph(engine.storage.as_ref(), 10_000);
    let root_id = ids[0];

    c.bench_function("traversal/out_node/medium", |b| {
        let storage = engine.storage.as_ref();
        b.iter(|| {
            let arena = Bump::new();
            let rtxn = storage.graph_env.read_txn().unwrap();
            let count = G::new(storage, &rtxn, &arena)
                .n_from_type("person")
                .out_node(black_box("knows"))
                .take(20_000)
                .count();
            black_box(count)
        })
    });

    let _ = root_id;
}

criterion_group!(
    benches,
    bench_n_from_type_small,
    bench_n_from_type_medium,
    bench_out_node_small,
    bench_out_node_medium
);
criterion_main!(benches);
