//! HQL compiler benchmarks.
//!
//! Three groups:
//!   - `compiler/parse`    — SparrowParser::parse_source only
//!   - `compiler/analyze`  — analyze() on an already-parsed Source
//!   - `compiler/full`     — parse + analyze round-trip
//!
//! Run with:
//!   cargo bench -p sparrow-benches --bench compiler --features cpu

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use sparrow_db::sparrowc::parser::SparrowParser;
use sparrow_db::sparrowc::analyzer::analyze;
use sparrow_benches::{HQL_SOURCE, make_content};

fn bench_parse(c: &mut Criterion) {
    let content = make_content(HQL_SOURCE);
    c.bench_function("compiler/parse", |b| {
        b.iter(|| SparrowParser::parse_source(black_box(&content)).unwrap())
    });
}

fn bench_analyze(c: &mut Criterion) {
    let content = make_content(HQL_SOURCE);
    let source = SparrowParser::parse_source(&content).unwrap();
    c.bench_function("compiler/analyze", |b| {
        b.iter(|| analyze(black_box(&source)).unwrap())
    });
}

fn bench_full_compile(c: &mut Criterion) {
    let content = make_content(HQL_SOURCE);
    c.bench_function("compiler/full", |b| {
        b.iter(|| {
            let source = SparrowParser::parse_source(black_box(&content)).unwrap();
            analyze(black_box(&source)).unwrap()
        })
    });
}

criterion_group!(benches, bench_parse, bench_analyze, bench_full_compile);
criterion_main!(benches);
