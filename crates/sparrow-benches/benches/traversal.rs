use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn traversal_benchmark(c: &mut Criterion) {
    c.bench_function("traversal_stub", |b| {
        b.iter(|| {
            black_box(1 + 1)
        });
    });
}

criterion_group!(benches, traversal_benchmark);
criterion_main!(benches);
