//! Write-pipeline benchmarks — serialise and deserialise record batches.
//!
//! This measures the encoding layer independently of LMDB commit latency.
//! Two groups:
//!   - `write_pipeline/serialize/<n>`   — `Node::to_bincode_bytes()` for batch sizes 1/100/1000/10000
//!   - `write_pipeline/deserialize/<n>` — `Node::from_bincode_bytes()` for the same sizes
//!
//! Run with:
//!   cargo bench -p sparrow-benches --bench write_pipeline --features cpu

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use bumpalo::Bump;
use sparrow_db::utils::items::Node;
use sparrow_db::utils::id::v6_uuid;

const BATCH_SIZES: &[usize] = &[1, 100, 1_000, 10_000];

// ---------------------------------------------------------------------------
// Serialisation
// ---------------------------------------------------------------------------

fn bench_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_pipeline/serialize");

    for &n in BATCH_SIZES {
        let arena = Bump::new();
        let nodes: Vec<Node<'_>> = (0..n)
            .map(|_| Node {
                id: v6_uuid(),
                label: arena.alloc_str("Person"),
                version: 1,
                properties: None,
            })
            .collect();

        group.bench_with_input(BenchmarkId::from_parameter(n), &nodes, |b, nodes| {
            b.iter(|| {
                nodes
                    .iter()
                    .map(|node| node.to_bincode_bytes().unwrap())
                    .collect::<Vec<_>>()
            })
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Deserialisation
// ---------------------------------------------------------------------------

fn bench_deserialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_pipeline/deserialize");

    for &n in BATCH_SIZES {
        let serialized: Vec<(u128, Vec<u8>)> = {
            let arena = Bump::new();
            (0..n)
                .map(|_| {
                    let id = v6_uuid();
                    let node = Node {
                        id,
                        label: arena.alloc_str("Person"),
                        version: 1,
                        properties: None,
                    };
                    (id, node.to_bincode_bytes().unwrap())
                })
                .collect()
        };

        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &serialized,
            |b, serialized| {
                b.iter(|| {
                    let arena = Bump::new();
                    let mut count: usize = 0;
                    for (id, bytes) in serialized {
                        // Argument order: id, bytes, arena
                        let node = Node::from_bincode_bytes(
                            black_box(*id),
                            black_box(bytes.as_slice()),
                            &arena,
                        )
                        .unwrap();
                        // prevent the compiler from eliding the work
                        count += node.id as usize & 1;
                    }
                    black_box(count)
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_serialize, bench_deserialize);
criterion_main!(benches);
