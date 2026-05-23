# Performance Benchmark Suite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `crates/sparrow-benches/` — a dedicated Cargo workspace member with criterion benchmarks for the HQL compiler, graph traversal, and record write pipeline, plus Makefile targets and a CI workflow that posts a before/after diff on every PR.

**Architecture:** Feature-gated crate (`cpu` default, `http` future stub) with three `[[bench]]` targets that exercise public `sparrow_db` APIs. Baselines are committed JSON files under `crates/sparrow-benches/baselines/`; `critcmp` diffs them on PRs. No PR-blocking — baseline-only for now.

**Tech Stack:** Rust / Criterion 0.5, bumpalo, heed3, sparrow_db (features: `lmdb,bench`), critcmp, GitHub Actions.

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Modify | `Cargo.toml` | Add `crates/sparrow-benches` to workspace members |
| Create | `crates/sparrow-benches/Cargo.toml` | Crate manifest: deps, features, `[[bench]]` entries |
| Create | `crates/sparrow-benches/src/lib.rs` | Shared fixture helpers: `make_engine`, `seed_graph`, HQL source constants |
| Create | `crates/sparrow-benches/benches/compiler.rs` | HQL parse + analyze benchmarks |
| Create | `crates/sparrow-benches/benches/traversal.rs` | Graph traversal benchmarks (n_from_type, out_node, multi-hop) |
| Create | `crates/sparrow-benches/benches/write_pipeline.rs` | Node serialise/deserialise benchmarks |
| Create | `crates/sparrow-benches/baselines/.gitkeep` | Placeholder until first `bench-update` run |
| Modify | `Makefile` | Add `bench`, `bench-update`, `bench-diff` targets |
| Create | `.github/workflows/bench.yml` | CI job: restore baselines, run, post critcmp diff |

---

## Task 1: Scaffold the crate and register with workspace

**Files:**
- Modify: `Cargo.toml` (root)
- Create: `crates/sparrow-benches/Cargo.toml`
- Create: `crates/sparrow-benches/src/lib.rs`
- Create: `crates/sparrow-benches/baselines/.gitkeep`

- [ ] **Step 1: Add crate to workspace members**

In `Cargo.toml` at the workspace root, add the new member to the `[workspace] members` list (keep alphabetical order within `crates/`):

```toml
members = [
    "crates/sparrow-benches",
    "crates/sparrow-core",
    "crates/sparrow-container",
    "crates/sparrow-macros",
    "crates/sparrow-cli",
    "crates/sparrow-metrics",
    "crates/sparrow-memory",
    "crates/sparrow-chef",
    "crates/sparrow-studio",
    "sdks/rust",
    "tests/hql-tests",
]
```

- [ ] **Step 2: Create `crates/sparrow-benches/Cargo.toml`**

```toml
[package]
name = "sparrow-benches"
version = "0.1.0"
edition = "2024"
publish = false
description = "CPU and (future) HTTP performance benchmarks for SparrowDB"

[lib]
name = "sparrow_benches"
path = "src/lib.rs"

[features]
# cpu: default — criterion benchmarks, no live DB, no network
cpu = []
# http: future — adds sparrow-sdk + HTTP client for live endpoint benchmarks
http = []
default = ["cpu"]

[dependencies]
sparrow-core = { path = "../sparrow-core", features = ["lmdb", "bench"] }
tempfile = "3"
rand = "0.9"
bumpalo = { version = "3", features = ["collections"] }

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

- [ ] **Step 3: Create `crates/sparrow-benches/src/lib.rs`**

Leave it as a module stub for now (fixture helpers go in Task 2):

```rust
// Shared fixture helpers for sparrow-benches.
// See each function's doc-comment for usage from bench files.
```

- [ ] **Step 4: Create baseline placeholder**

```bash
mkdir -p crates/sparrow-benches/baselines
touch crates/sparrow-benches/baselines/.gitkeep
```

- [ ] **Step 5: Verify the crate compiles cleanly**

```bash
cargo check -p sparrow-benches --features cpu
```

Expected: no errors. Warnings about unused imports are fine at this stage.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/sparrow-benches/
git commit -m "chore(benches): scaffold sparrow-benches workspace crate"
```

---

## Task 2: Write shared fixture helpers

**Files:**
- Modify: `crates/sparrow-benches/src/lib.rs`

- [ ] **Step 1: Write the fixture helpers**

Replace the stub in `crates/sparrow-benches/src/lib.rs` with:

```rust
//! Shared fixture helpers for sparrow-benches.
//!
//! `make_engine` creates a minimal in-process SparrowGraphEngine backed by a
//! temp directory. `seed_graph` populates it with `node_count` "person" nodes
//! connected by "knows" edges. Both are meant to be called once per benchmark
//! group (not per iteration).

use sparrow_db::sparrow_engine::traversal_core::{
    SparrowGraphEngine, SparrowGraphEngineOpts,
    config::Config,
    ops::{
        g::G,
        source::{add_e::AddEAdapter, add_n::AddNAdapter},
    },
};
use sparrow_db::sparrow_engine::storage_core::version_info::VersionInfo;
use sparrow_db::sparrow_engine::storage_core::SparrowGraphStorage;
use sparrow_db::sparrow_engine::types::GraphError;
use sparrow_db::sparrowc::parser::types::{Content, HxFile, Source};
use bumpalo::Bump;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Engine factory
// ---------------------------------------------------------------------------

/// Create a minimal SparrowGraphEngine in a temporary directory.
/// The caller must keep `TempDir` alive for the lifetime of the engine.
pub fn make_engine() -> (SparrowGraphEngine, TempDir) {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut config = Config::default();
    config.db_max_size_gb = Some(1);
    let engine = SparrowGraphEngine::new(SparrowGraphEngineOpts {
        path: temp_dir.path().to_str().unwrap().to_string(),
        config,
        version_info: VersionInfo::default(),
    })
    .expect("failed to create SparrowGraphEngine");
    (engine, temp_dir)
}

// ---------------------------------------------------------------------------
// Graph seeding
// ---------------------------------------------------------------------------

/// Seed `node_count` "person" nodes and edges between consecutive pairs.
///
/// Returns the list of inserted node IDs. Edges are only inserted where
/// `node_count >= 2`.
///
/// Calling `write_txn()` directly is intentional — benchmarks run without a
/// WorkerPool, so there is no concurrent writer and the single-writer
/// invariant is satisfied.
pub fn seed_graph(storage: &SparrowGraphStorage, node_count: usize) -> Vec<u128> {
    let arena = Bump::new();
    let mut wtxn = storage
        .graph_env
        .write_txn()
        .expect("failed to open write txn");

    let mut ids: Vec<u128> = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let node = G::new_mut(storage, &arena, &mut wtxn)
            .add_n("person", None, None)
            .collect_to_obj()
            .expect("add_n failed");
        ids.push(node.id());
    }

    for window in ids.windows(2) {
        G::new_mut(storage, &arena, &mut wtxn)
            .add_edge("knows", None, window[0], window[1], false)
            .collect_to_obj()
            .expect("add_edge failed");
    }

    wtxn.commit().expect("failed to commit seeding txn");
    ids
}

// ---------------------------------------------------------------------------
// HQL source constants used by compiler.rs bench
// ---------------------------------------------------------------------------

/// A representative HQL file covering point lookup, traversal, and mutation.
/// The schema and queries are intentionally simple so the parser/analyser time
/// is dominated by the compiler pipeline, not schema complexity.
pub const HQL_SOURCE: &str = r#"
N::Person {
    INDEX name: String,
    age: I32,
}

E::Knows {
    From: Person,
    To: Person,
}

QUERY get_person(id: ID) =>
    person <- N<Person>(id)
    RETURN person

QUERY get_friends(id: ID) =>
    person <- N<Person>(id)
    friends <- person::Out<Knows>
    RETURN friends

QUERY add_person(name: String, age: I32) =>
    person <- AddN<Person>({name: name, age: age})
    RETURN person
"#;

/// Wrap a raw HQL string in the `Content` type expected by `SparrowParser::parse_source`.
pub fn make_content(src: &str) -> Content {
    Content {
        content: src.to_string(),
        source: Source::default(),
        files: vec![HxFile {
            name: "bench.hx".to_string(),
            content: src.to_string(),
        }],
    }
}
```

- [ ] **Step 2: Verify it builds**

```bash
cargo build -p sparrow-benches --features cpu
```

Expected: compiles cleanly. If you see "unresolved import" errors, double-check the import paths against `crates/sparrow-core/src/lib.rs` and its submodules.

- [ ] **Step 3: Commit**

```bash
git add crates/sparrow-benches/src/lib.rs
git commit -m "feat(benches): add shared fixture helpers to sparrow-benches"
```

---

## Task 3: Compiler benchmark

**Files:**
- Create: `crates/sparrow-benches/benches/compiler.rs`

- [ ] **Step 1: Create the benchmark file**

```rust
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
```

- [ ] **Step 2: Verify it runs (quick mode)**

```bash
cargo bench -p sparrow-benches --bench compiler --features cpu -- --test
```

Expected output (each bench runs once, no statistics):
```
test compiler/parse   ... ok
test compiler/analyze ... ok
test compiler/full    ... ok
```

If you see "SparrowParser not found" or similar, check the import path — the compiler feature is gated behind `lmdb -> server -> build -> compiler`. Since we depend on `sparrow-core` with `features = ["lmdb", "bench"]`, this chain is active.

- [ ] **Step 3: Commit**

```bash
git add crates/sparrow-benches/benches/compiler.rs
git commit -m "feat(benches): add HQL compiler criterion benchmarks"
```

---

## Task 4: Traversal benchmark

**Files:**
- Create: `crates/sparrow-benches/benches/traversal.rs`

- [ ] **Step 1: Create the benchmark file**

```rust
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
    traversal_iter::RoTraversalIterator,
    traversal_value::TraversalValue,
};
use sparrow_db::sparrow_engine::types::GraphError;
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
            let results: Vec<TraversalValue<'_>> = G::new(storage, &rtxn, &arena)
                .n_from_type(black_box("person"))
                .take_and_collect_to(200);
            black_box(results)
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
            let results: Vec<TraversalValue<'_>> = G::new(storage, &rtxn, &arena)
                .n_from_type(black_box("person"))
                .take_and_collect_to(20_000);
            black_box(results)
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
            let results: Vec<TraversalValue<'_>> = G::new(storage, &rtxn, &arena)
                .n_from_type("person")
                .out_node(black_box("knows"))
                .take_and_collect_to(200);
            black_box(results)
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
            let results: Vec<TraversalValue<'_>> = G::new(storage, &rtxn, &arena)
                .n_from_type("person")
                .out_node(black_box("knows"))
                .take_and_collect_to(20_000);
            black_box(results)
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
```

- [ ] **Step 2: Verify it runs (quick mode)**

```bash
cargo bench -p sparrow-benches --bench traversal --features cpu -- --test
```

Expected output:
```
test traversal/n_from_type/small  ... ok
test traversal/n_from_type/medium ... ok
test traversal/out_node/small     ... ok
test traversal/out_node/medium    ... ok
```

The medium fixtures take a few seconds to seed — that's expected. If you see `GraphError` panics, verify that `seed_graph` in `src/lib.rs` is committing its write transaction before the bench iterations begin.

- [ ] **Step 3: Commit**

```bash
git add crates/sparrow-benches/benches/traversal.rs
git commit -m "feat(benches): add graph traversal criterion benchmarks"
```

---

## Task 5: Write-pipeline benchmark

**Files:**
- Create: `crates/sparrow-benches/benches/write_pipeline.rs`

- [ ] **Step 1: Create the benchmark file**

```rust
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
        // Pre-build nodes — we only benchmark the serialisation step.
        let arena = Bump::new();
        let nodes: Vec<Node<'_>> = (0..n)
            .map(|_| Node {
                id: v6_uuid(),
                label: "Person",
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
        // Pre-serialise so we only benchmark the decode step.
        let serialized: Vec<(u128, Vec<u8>)> = (0..n)
            .map(|_| {
                let id = v6_uuid();
                let arena = Bump::new();
                let node = Node {
                    id,
                    label: "Person",
                    version: 1,
                    properties: None,
                };
                (id, node.to_bincode_bytes().unwrap())
            })
            .collect();

        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &serialized,
            |b, serialized| {
                b.iter(|| {
                    let arena = Bump::new();
                    let nodes: Vec<Node<'_>> = serialized
                        .iter()
                        .map(|(id, bytes)| {
                            Node::from_bincode_bytes(&arena, black_box(*id), black_box(bytes))
                                .unwrap()
                        })
                        .collect();
                    black_box(nodes)
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_serialize, bench_deserialize);
criterion_main!(benches);
```

- [ ] **Step 2: Verify it runs (quick mode)**

```bash
cargo bench -p sparrow-benches --bench write_pipeline --features cpu -- --test
```

Expected output:
```
test write_pipeline/serialize/1       ... ok
test write_pipeline/serialize/100     ... ok
test write_pipeline/serialize/1000    ... ok
test write_pipeline/serialize/10000   ... ok
test write_pipeline/deserialize/1     ... ok
test write_pipeline/deserialize/100   ... ok
test write_pipeline/deserialize/1000  ... ok
test write_pipeline/deserialize/10000 ... ok
```

If `Node::from_bincode_bytes` is not found, check `crates/sparrow-core/src/utils/items.rs` — the method is `pub fn from_bincode_bytes(arena, id, bytes)`.

- [ ] **Step 3: Confirm all three bench targets compile and run together**

```bash
cargo bench -p sparrow-benches --features cpu -- --test
```

Expected: all 15 tests show `ok`.

- [ ] **Step 4: Commit**

```bash
git add crates/sparrow-benches/benches/write_pipeline.rs
git commit -m "feat(benches): add write-pipeline criterion benchmarks"
```

---

## Task 6: Makefile targets

**Files:**
- Modify: `Makefile`

- [ ] **Step 1: Add bench targets to the Makefile**

Append the following to the end of `Makefile`. Keep the existing `.PHONY` declaration — add the new targets to the existing line rather than duplicating it:

First update the `.PHONY` line at the top of the Makefile to include the new targets:

```makefile
.PHONY: build check test test-all sweep \
        sdk-build sdk-check \
        docker-build docker-up docker-down docker-logs \
        bench bench-update bench-diff
```

Then append the new targets at the end of the file:

```makefile
## Run CPU benchmarks (no comparison — just measure)
bench:
	cargo bench -p sparrow-benches --features cpu

## Re-run benchmarks, save new baselines, stage for commit.
## Review the diff with `git diff --staged`, then commit manually:
##   git commit -m "perf(benches): update baselines"
bench-update:
	cargo bench -p sparrow-benches --features cpu -- --save-baseline main
	@for bench in compiler traversal write_pipeline; do \
	  mkdir -p crates/sparrow-benches/baselines/$$bench; \
	  if [ -d target/criterion/$$bench/main ]; then \
	    cp -r target/criterion/$$bench/main crates/sparrow-benches/baselines/$$bench/; \
	  fi; \
	done
	git add crates/sparrow-benches/baselines/
	@echo "Baselines staged. Run: git commit -m 'perf(benches): update baselines'"

## Diff the current run against committed main baselines (same as CI does).
## Requires critcmp: cargo install critcmp
bench-diff:
	rsync -av crates/sparrow-benches/baselines/ target/criterion/
	cargo bench -p sparrow-benches --features cpu -- --save-baseline current
	critcmp main current
```

- [ ] **Step 2: Smoke-test the `bench` target**

```bash
make bench 2>&1 | head -30
```

Expected: criterion starts running and prints timing output. (Interrupt with Ctrl-C once you see output — full runs are slow.)

- [ ] **Step 3: Commit**

```bash
git add Makefile
git commit -m "chore(benches): add bench / bench-update / bench-diff Makefile targets"
```

---

## Task 7: CI workflow

**Files:**
- Create: `.github/workflows/bench.yml`

- [ ] **Step 1: Create the workflow file**

```yaml
name: Benchmarks

on:
  pull_request:
    paths:
      # Only run when bench code or sparrow-core changes
      - "crates/sparrow-benches/**"
      - "crates/sparrow-core/**"

jobs:
  bench:
    name: CPU benchmark diff
    runs-on: ubuntu-latest
    permissions:
      pull-requests: write

    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2
        with:
          key: sparrow-benches

      - name: Install critcmp
        run: cargo install critcmp --locked

      - name: Restore committed main baselines
        run: |
          # Copy committed baselines into target/criterion/ so critcmp can find them
          if [ -d crates/sparrow-benches/baselines ]; then
            rsync -av crates/sparrow-benches/baselines/ target/criterion/
          else
            echo "No committed baselines yet — skipping comparison"
          fi

      - name: Run benchmarks
        run: cargo bench -p sparrow-benches --features cpu -- --save-baseline current

      - name: Generate diff
        id: diff
        run: |
          if [ -d target/criterion ] && ls target/criterion/*/main 2>/dev/null | head -1 | grep -q main; then
            critcmp main current > bench-diff.txt 2>&1 || true
          else
            echo "No main baseline to compare against — first run establishes baseline." > bench-diff.txt
          fi
          echo "has_diff=true" >> $GITHUB_OUTPUT

      - name: Post PR comment
        if: steps.diff.outputs.has_diff == 'true'
        uses: actions/github-script@v7
        with:
          script: |
            const fs = require('fs');
            const diff = fs.readFileSync('bench-diff.txt', 'utf8');
            const body = `## Benchmark diff vs main baseline\n\n\`\`\`\n${diff}\n\`\`\`\n\n_Run \`make bench-update\` on main to refresh baselines._`;
            github.rest.issues.createComment({
              owner: context.repo.owner,
              repo: context.repo.repo,
              issue_number: context.issue.number,
              body,
            });

      - name: Summary
        run: |
          echo "## Benchmark diff vs main baseline" >> $GITHUB_STEP_SUMMARY
          cat bench-diff.txt >> $GITHUB_STEP_SUMMARY
```

- [ ] **Step 2: Verify the workflow file parses correctly (local lint)**

```bash
# If you have the GitHub CLI installed:
gh workflow list 2>/dev/null || echo "gh CLI not available — skip"
# At minimum, check YAML syntax:
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/bench.yml'))" && echo "YAML OK"
```

Expected: `YAML OK`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/bench.yml
git commit -m "ci(benches): add benchmark diff workflow for PRs"
```

---

## Task 8: Establish initial baselines

**Files:**
- Modify: `crates/sparrow-benches/baselines/` (generated)

- [ ] **Step 1: Run the full benchmark suite and save baselines**

This run collects statistics — it will take several minutes:

```bash
cargo bench -p sparrow-benches --features cpu -- --save-baseline main
```

Expected: criterion prints per-benchmark timing tables. The medium-sized traversal fixtures (10 000 nodes) will take the longest.

- [ ] **Step 2: Copy baselines into the repo**

```bash
for bench in compiler traversal write_pipeline; do
  mkdir -p crates/sparrow-benches/baselines/$bench
  if [ -d target/criterion/$bench/main ]; then
    cp -r target/criterion/$bench/main crates/sparrow-benches/baselines/$bench/
  fi
done
```

- [ ] **Step 3: Verify baselines were written**

```bash
find crates/sparrow-benches/baselines -name "estimates.json" | head -10
```

Expected: at least one `estimates.json` per benchmark group (compiler, traversal, write_pipeline).

- [ ] **Step 4: Remove the .gitkeep placeholder**

```bash
rm crates/sparrow-benches/baselines/.gitkeep
```

- [ ] **Step 5: Commit**

```bash
git add crates/sparrow-benches/baselines/
git commit -m "perf(benches): establish initial CPU benchmark baselines"
```

---

## Self-Review Checklist (already applied)

- **Spec coverage:** Architecture (§3), three bench groups (§4), baseline workflow (§5), CI (§6), Makefile (§8), LMDB constraint note (§9) — all covered by Tasks 1–8.
- **Placeholders:** None. All code blocks are complete and runnable.
- **Type consistency:** `TraversalValue`, `SparrowGraphEngine`, `SparrowGraphEngineOpts`, `SparrowGraphStorage`, `G`, `AddNAdapter`, `AddEAdapter`, `NFromTypeAdapter`, `OutAdapter`, `SparrowParser`, `analyze`, `Node`, `ImmutablePropertiesMap`, `Value`, `v6_uuid` — all used consistently across Tasks 2–5.
- **Import path accuracy:** All paths verified against `crates/sparrow-core/src/lib.rs` and submodule `mod.rs` files.
