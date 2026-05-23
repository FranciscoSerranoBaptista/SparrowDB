use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn compiler_benchmark(c: &mut Criterion) {
    c.bench_function("compiler_stub", |b| {
        b.iter(|| {
            black_box(1 + 1)
        });
    });
}

criterion_group!(benches, compiler_benchmark);
criterion_main!(benches);
