# sparrow-benches

Criterion-based performance benchmarks for SparrowDB. Measures the three
CPU-hot paths — HQL compiler, graph traversal, and the record
serialisation/deserialisation pipeline — with no live HTTP server or network
required.

## Structure

```
crates/sparrow-benches/
  Cargo.toml           # workspace member; feature flags: cpu (default), http (future)
  src/
    lib.rs             # shared fixture helpers: make_engine, seed_graph, HQL constants
  benches/
    compiler.rs        # HQL parse → analysis plan
    traversal.rs       # in-LMDB graph walk (n_from_type, out_node)
    write_pipeline.rs  # Node::to_bincode_bytes / from_bincode_bytes
  baselines/           # committed criterion JSON — one sub-dir per benchmark ID
```

## Running benchmarks

```bash
# Run all benchmarks (takes ~5 min for full statistical collection)
make bench

# Quick sanity-check — each benchmark runs once, no timing collected
cargo bench -p sparrow-benches --bench compiler --bench traversal --bench write_pipeline \
  --features cpu -- --test
```

## Comparing against the committed baseline

Requires [`critcmp`](https://github.com/BurntSushi/critcmp):

```bash
cargo install critcmp --locked
make bench-diff
```

`make bench-diff` copies the committed baselines into `target/criterion/`, runs a
fresh measurement saved as `current`, then calls `critcmp main current` to print
a before/after table.

## Updating the baseline

After a known-good performance improvement lands on `main`:

```bash
make bench-update     # runs benchmarks, copies new JSONs to baselines/, stages them
git commit -m "perf(benches): update baselines"
```

## Benchmark groups

| Target | Groups | What it measures |
|--------|--------|-----------------|
| `compiler` | `compiler/parse`, `compiler/analyze`, `compiler/full` | HQL string → parsed AST → analysed schema (no LMDB) |
| `traversal` | `traversal/n_from_type/{small,medium}`, `traversal/out_node/{small,medium}` | Node scan and single-hop edge walk over LMDB fixtures of 100 and 10 000 nodes |
| `write_pipeline` | `write_pipeline/serialize/{1,100,1000,10000}`, `write_pipeline/deserialize/{1,100,1000,10000}` | `Node::to_bincode_bytes` / `from_bincode_bytes` at batch sizes 1–10 000 |

## Baseline numbers (2026-05-23, macOS M-series)

| Benchmark | Median |
|-----------|--------|
| `compiler/parse` | 61.7 µs |
| `compiler/analyze` | 7.4 µs |
| `compiler/full` | 69.4 µs |
| `traversal/n_from_type/small` (100 nodes) | 3.1 µs |
| `traversal/n_from_type/medium` (10 k nodes) | 264 µs |
| `traversal/out_node/small` | 32 µs |
| `traversal/out_node/medium` | 5.1 ms |
| `write_pipeline/serialize/1` | 15 ns |
| `write_pipeline/serialize/10000` | 101 µs |
| `write_pipeline/deserialize/1` | 20 ns |
| `write_pipeline/deserialize/10000` | 102 µs |

## CI integration

`.github/workflows/bench.yml` runs on every PR that touches
`crates/sparrow-benches/**` or `crates/sparrow-core/**`. It:

1. Restores committed baselines into `target/criterion/`.
2. Runs a full benchmark pass saved as the `current` baseline.
3. Posts a `critcmp main current` diff as a PR comment and step summary.

No PR is blocked — the diff is informational only.

## Known issues

### `seed_graph` bypasses `add_n` / `add_edge`

The traversal fixture helper writes directly to the LMDB databases instead of
going through the traversal-iterator API. This is because `add_n` and
`add_edge` use `PutFlags::APPEND`, which requires strictly ascending u128
keys — a guarantee that `v6_uuid()` can break when called in a tight loop on
a machine where the OS clock resolution is coarser than the UUID timestamp
tick (100 ns).

The workaround: pre-generate all IDs, sort them, then write with `put()`.
Full write-up: [`docs/superpowers/known-issues.md`](../../docs/superpowers/known-issues.md).

## Feature flags

| Feature | Description |
|---------|-------------|
| `cpu` (default) | Criterion benchmarks — no live DB, no network |
| `http` | Future: HTTP round-trip benchmarks against a running container (not yet implemented) |
