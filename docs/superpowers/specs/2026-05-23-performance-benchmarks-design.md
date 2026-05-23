# SparrowDB Performance Benchmark Suite — Design Spec

**Date:** 2026-05-23  
**Status:** Approved  
**Motivation:** C — CPU unit benchmarks now to catch regressions; HTTP integration benchmarks later once the container harness is ready.

---

## 1. Goals

- Establish baseline measurements for the three CPU hot paths: HQL compiler, graph traversal, and the write/serialisation pipeline.
- Make regressions visible on every PR via a before/after `critcmp` diff posted as a GitHub Actions annotation — no PR blocking for now.
- Provide a clear extension point for HTTP integration benchmarks (endpoint round-trips, full ingest pipeline, `SparrowClient` read/write) that require a live container.

## 2. Non-Goals

- Hard-gating PRs on benchmark regressions (revisit once baselines are stable).
- Migrating the existing `crates/sparrow-core/benches/` BM25 and HNSW bench files (they stay as-is for now).
- HTTP integration benchmarks (stubbed as a feature flag slot only).

---

## 3. New Crate: `crates/sparrow-benches/`

A dedicated Cargo workspace member containing all performance benchmarks. No binary; just `[[bench]]` targets plus a thin `src/lib.rs` for shared fixture helpers.

### 3.1 Directory layout

```
crates/sparrow-benches/
  Cargo.toml
  src/
    lib.rs              # shared fixture helpers (seeded graphs, sample records, query strings)
  benches/
    compiler.rs         # HQL parse → execution plan
    traversal.rs        # in-memory graph traversal over fixture graph
    write_pipeline.rs   # record serialise/deserialise + batch build (no LMDB commit)
  baselines/            # committed criterion JSON — one sub-dir per benchmark group
    compiler/
    traversal/
    write_pipeline/
```

### 3.2 `Cargo.toml`

```toml
[package]
name = "sparrow-benches"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
name = "sparrow_benches"
path = "src/lib.rs"

[features]
# cpu: default — criterion benchmarks, no live DB, no network
cpu = []
# http: future — adds sparrow-sdk + reqwest for live HTTP benchmarks
http = []
default = ["cpu"]

[dependencies]
sparrow-core = { path = "../sparrow-core", features = ["lmdb", "bench"] }
tempfile = "3"
rand = "0.9"

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "compiler"
harness = false
required-features = ["cpu"]

[[bench]]
name = "traversal"
harness = false
required-features = ["cpu"]

[[bench]]
name = "write_pipeline"
harness = false
required-features = ["cpu"]
```

> **Note:** `sparrow-core` is in `[dependencies]` (not `[dev-dependencies]`) because `src/lib.rs` fixture helpers also use it.  
> The `[lib] name = "sparrow_db"` override in `crates/sparrow-core/Cargo.toml` means all imports use `use sparrow_db::...`.

### 3.3 Add to workspace

In the root `Cargo.toml`, add `"crates/sparrow-benches"` to the `[workspace] members` list.

---

## 4. Benchmark Groups

### 4.1 `compiler.rs` — HQL parse → plan

Exercises the HQL compiler (feature `compiler` → pulled in via `lmdb` → `server` → `build` → `compiler`).

**Parametrised over representative query shapes:**
- Point lookup by ID
- Single-hop traversal with a filter predicate
- Multi-hop traversal with aggregation
- Full-text / BM25 query

Each input is a static query string. No LMDB, no network. Measures the time from raw string to a fully compiled execution plan.

```rust
// sketch
fn bench_compiler(c: &mut Criterion) {
    let mut group = c.benchmark_group("compiler");
    for (name, query) in QUERIES {
        group.bench_with_input(name, query, |b, q| {
            b.iter(|| sparrow_db::sparrowc::compile(black_box(q)))
        });
    }
}
```

### 4.2 `traversal.rs` — in-memory graph traversal

Seeds a fixture graph into a `tempfile`-backed LMDB env, then benchmarks graph walk operations without the HTTP or worker-pool overhead.

**Fixture sizes:** small (100 nodes, 200 edges), medium (10k nodes, 50k edges).

**Traversal shapes:**
- BFS from a root node, depth 3
- Filtered edge traversal (select edges by type)
- Reverse traversal (in-edges)

The LMDB env is created once per benchmark group (`criterion::BenchmarkGroup::setup`), not per iteration, so I/O is excluded from the measured time.

### 4.3 `write_pipeline.rs` — serialise / deserialise + batch build

Exercises the record encoding path: construct `protocol::Value` objects, serialise them via `bincode`, and deserialise back — without opening a write transaction. Measures throughput regressions in the encoding layer independently of LMDB commit latency.

**Parametrised over batch sizes:** 1, 100, 1 000, 10 000 records.

---

## 5. Baseline Workflow

### 5.1 Storing baselines

Criterion saves measurement JSON to `target/criterion/<bench_name>/<baseline_name>/`. Baselines committed to the repo live in `crates/sparrow-benches/baselines/` and mirror that structure:

```
baselines/
  compiler/main/estimates.json
  traversal/main/estimates.json
  write_pipeline/main/estimates.json
```

### 5.2 Updating baselines (on `main`)

```bash
make bench-update
# expands to:
# cargo bench -p sparrow-benches --features cpu -- --save-baseline main
# rsync -av --delete target/criterion/ crates/sparrow-benches/baselines/
# git add crates/sparrow-benches/baselines/
# git commit -m "perf(benches): update baselines [skip ci]"
```

Engineers run this intentionally after a known-good performance improvement lands on `main`.

### 5.3 Comparing on a PR

```bash
# Restore committed baselines into target/criterion/ so critcmp can find them
rsync -av crates/sparrow-benches/baselines/ target/criterion/

# Run benches and save as "current"
cargo bench -p sparrow-benches --features cpu -- --save-baseline current

# Diff — output is Markdown-friendly
critcmp main current
```

CI posts the `critcmp` output as a PR comment (via `gh pr comment`). No blocking — purely informational.

---

## 6. CI Integration

Add a new GitHub Actions job `bench` to `.github/workflows/` (or extend an existing workflow):

```yaml
bench:
  runs-on: ubuntu-latest
  if: github.event_name == 'pull_request'
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - name: Install critcmp
      run: cargo install critcmp --locked
    - name: Restore baselines
      run: rsync -av crates/sparrow-benches/baselines/ target/criterion/
    - name: Run benchmarks
      run: cargo bench -p sparrow-benches --features cpu -- --save-baseline current
    - name: Diff vs main baseline
      run: |
        critcmp main current > bench-diff.txt || true
        echo "## Benchmark diff vs main baseline" >> $GITHUB_STEP_SUMMARY
        cat bench-diff.txt >> $GITHUB_STEP_SUMMARY
    - name: Post PR comment
      run: |
        gh pr comment ${{ github.event.pull_request.number }} \
          --body-file bench-diff.txt
      env:
        GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

---

## 7. Future: `http` Feature Phase

When the container harness is ready, the `http` feature slot in `Cargo.toml` is activated. It adds:

- `sparrow-sdk` (workspace path dep) to `[dependencies]` behind `cfg(feature = "http")`
- A `benches/http.rs` file with a Tokio-runtime fixture that starts the container via `docker compose up`, waits for health, runs criterion HTTP benchmarks, tears down
- Separate baseline files under `baselines/http/`

No implementation now — the feature flag and file are stubbed only, with a `compile_error!` guard if the `http` feature is activated without the container env var set.

---

## 8. Makefile Targets

Add to the root `Makefile`:

```makefile
.PHONY: bench bench-update bench-diff

## Run CPU benchmarks (no baseline comparison)
bench:
	cargo bench -p sparrow-benches --features cpu

## Re-run benchmarks and commit new baselines to the repo
bench-update:
	cargo bench -p sparrow-benches --features cpu -- --save-baseline main
	@for bench in compiler traversal write_pipeline; do \
	  mkdir -p crates/sparrow-benches/baselines/$$bench; \
	  cp -r target/criterion/$$bench/main crates/sparrow-benches/baselines/$$bench/ 2>/dev/null || true; \
	done
	git add crates/sparrow-benches/baselines/
	@echo "Baselines updated. Review the diff then: git commit -m 'perf(benches): update baselines'"

## Diff current run vs committed main baseline (same as CI does)
bench-diff:
	rsync -av crates/sparrow-benches/baselines/ target/criterion/
	cargo bench -p sparrow-benches --features cpu -- --save-baseline current
	critcmp main current
```

---

## 9. Constraints Inherited from `CLAUDE.md`

- Crate lives under `crates/` per workspace convention.
- All imports use `use sparrow_db::...` (lib name override in sparrow-core).
- No `std::process::Command` in async code — benchmark setup helpers use synchronous code only; any async fixtures use `tokio::process::Command`.
- LMDB traversal benchmarks seed data by opening `write_txn()` directly on the LMDB env — this is safe in a benchmark context because no WorkerPool is running, so there is no concurrent writer. The single-writer invariant in `CLAUDE.md` applies to production server code, not to isolated benchmark or test setups. The existing `bm25_benches.rs` follows this same pattern.
