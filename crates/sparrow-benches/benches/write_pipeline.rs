use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn write_pipeline_benchmark(c: &mut Criterion) {
    c.bench_function("write_pipeline_stub", |b| {
        b.iter(|| {
            black_box(1 + 1)
        });
    });
}

criterion_group!(benches, write_pipeline_benchmark);
criterion_main!(benches);
