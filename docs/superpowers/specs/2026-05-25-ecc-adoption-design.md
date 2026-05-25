# SparrowDB — ECC Adoption Design

**Date**: 2026-05-25
**Status**: Approved
**Scope**: Surgical cherry-pick of ECC agents, hooks, and skills for the performance/memory bug-hunting phase

---

## Context

SparrowDB is currently in a performance and memory bug-hunting phase. The dominant work is:
1. **Profiling & benchmarking** — CPU hotspots, heap allocation, HNSW/LMDB throughput
2. **Hunting silent bugs** — swallowed errors in async tasks, HNSW soft-delete accumulation, LMDB write transaction leaks, `unwrap()` panics in storage paths
3. **Fixing known issues** — executing targeted repairs without introducing regressions

ECC (github.com/FranciscoSerranoBaptista/ECC) is a harness-native operator system with 60+ agents and 200+ skills. Most are web/JS-focused and not relevant to a Rust embedded graph-DB engine. This design cherry-picks what directly serves the current phase and adds one custom agent not in ECC.

---

## Deliverables

```
.agents/
  rust-reviewer.md            ← ECC base + SparrowDB invariant augmentations
  rust-build-resolver.md      ← ECC base + workspace context
  silent-failure-hunter.md    ← ECC, direct use
  sparrow-perf-profiler.md    ← NEW, custom to SparrowDB

docs/skills/
  setup.md                    ← complete from 2026-05-23 spec (Task 2)
  migration.md                ← complete from 2026-05-23 spec (Task 3)
  debugging.md                ← complete from 2026-05-23 spec (Task 4) — highest priority

scripts/
  post-build-sweep.sh         ← existing; stays here, path unchanged
scripts/hooks/
  rust-async-guard.sh         ← warns on std::process::Command in edited .rs files
  cargo-panic-extractor.sh    ← extracts panic/OOM/error signals from cargo test output

.claude/settings.json         ← NEW committed project-level hook wiring
CLAUDE.md                     ← add §Profiling tools + §Agent invocation guide

crates/sparrow-core/CLAUDE.md
crates/sparrow-container/CLAUDE.md
crates/sparrow-cli/CLAUDE.md
crates/sparrow-macros/CLAUDE.md
sdks/rust/CLAUDE.md
tests/hql-tests/CLAUDE.md
```

---

## Agent Designs

### `.agents/rust-reviewer.md`

Base: ECC `agents/rust-reviewer.md` (runs `cargo check`, `clippy`, `fmt`, `test` before diff analysis).

**SparrowDB augmentations — added as a top-priority block before the ECC CRITICAL list:**

CRITICAL (blocking, SparrowDB-specific):
- `std::process::Command` inside any `async fn` → use `tokio::process::Command`; this blocks the Tokio thread pool and has caused production hangs (see root CLAUDE.md)
- `write_txn()` opened outside the `WorkerPool` writer thread → LMDB single-writer violation; will deadlock or corrupt under concurrent load
- `use sparrow_core::` as the import path → the lib name override means the correct import is always `use sparrow_db::`

HIGH (SparrowDB-specific):
- `DROP` on a node in write-heavy code without a documented re-index plan → HNSW soft-delete accumulation degrades recall over time
- `?` or `.unwrap()` that converts `GraphError` to a generic error, losing the error variant → `GraphError` must be propagated with full context (pattern established in commits d2907985, 9013c50a)

Tools available to agent: `Read`, `Grep`, `Glob`, `Bash`
Model: Sonnet

---

### `.agents/rust-build-resolver.md`

Base: ECC `agents/rust-build-resolver.md` (sequential: `cargo check` → examine → fix → re-check → `clippy` → test).

**SparrowDB workspace augmentations:**

- `[lib] name = "sparrow_db"` in `crates/sparrow-core/Cargo.toml` is intentional and must never be removed; removing it requires updating every `use sparrow_db::` import across sparrow-cli, sparrow-container, sparrow-memory
- Feature flag chain: `lmdb` pulls in `server` which pulls in `build + compiler + vectors`; missing flags cause confusing "feature not found" errors
- Tests that touch the graph or gateway require `--features lmdb,server`; the default feature `lmdb` includes `server` but explicit flags are safer in CI
- Ariadne crate must be present when the `compiler` feature is active; its absence causes HQL error formatting to fail silently

Tools: `Read`, `Grep`, `Glob`, `Bash`
Model: Sonnet

---

### `.agents/silent-failure-hunter.md`

Base: ECC `agents/silent-failure-hunter.md`, direct use. No SparrowDB augmentations needed — the five hunt categories (empty catch, inadequate logging, dangerous fallbacks, error propagation issues, missing error handling) map directly to the storage engine failure patterns being tracked.

Particularly valuable for:
- LMDB transaction error propagation (errors returned from `heed3` must not be silently mapped to `GraphError::Unknown`)
- HNSW operation results (insert/delete failures that return `Ok(())` instead of surfacing the error)
- Async task join handles where the result is dropped without inspection

Tools: `Read`, `Grep`, `Glob`, `Bash`
Model: Sonnet

---

### `.agents/sparrow-perf-profiler.md`

New, custom agent. A four-phase workflow for profiling SparrowDB performance and memory issues.

**Phase 1 — Measure baseline**
```bash
sparrow stress <instance>                          # built-in load test
cargo criterion --bench <name>                     # microbenchmark baseline
RUST_LOG=sparrow_db=debug sparrow run              # runtime log with timing
curl -H "x-api-key: $TOKEN" localhost:6969/diagnostics  # HNSW health snapshot
```
Record: p50/p95/p99 latency, soft_deleted count, active vector count, entry_point_present.

**Phase 2 — Locate hotspot**
```bash
cargo flamegraph --bin sparrow-container           # CPU flame graph (install: cargo install flamegraph)
cargo +nightly dhat --bin sparrow-container        # heap allocation by call site
# Linux only:
heaptrack ./target/debug/sparrow-container         # live heap growth
# Isolate BM25 rebuild cost:
SPARROW_SKIP_BM25_ON_WRITE=1 sparrow run           # skip BM25 on writes; compare latency
# Trigger manual BM25 rebuild and time it:
time curl -X POST -H "x-api-key: $TOKEN" localhost:6969/rebuild_bm25_index
```

**Phase 3 — Hypothesis + targeted fix**
- HNSW soft-delete ratio > 20%: plan re-embed into fresh vector type (no in-place compaction)
- LMDB write amplification: check `WorkerPool` queue depth; batch writes where possible; consider `BatchAddV` for vector inserts
- Tokio thread starvation: `RUST_LOG=tokio=trace` → look for blocked tasks; grep for `std::process::Command` in async paths
- BM25 rebuild dominating write latency: use `SPARROW_SKIP_BM25_ON_WRITE=1` in write-heavy phases; schedule `/rebuild_bm25_index` async
- Dispatches `rust-reviewer` for any code changes; `silent-failure-hunter` if a fix touches error handling paths

**Phase 4 — Confirm improvement**
```bash
cargo criterion --bench <same-bench>               # compare vs baseline
curl .../diagnostics                               # confirm soft_deleted improved
sparrow stress <instance>                          # end-to-end confirmation
```
Document delta: p99 latency before vs after, memory growth rate before vs after, soft_deleted ratio.

Tools: `Read`, `Grep`, `Glob`, `Bash`, `Agent` (to dispatch rust-reviewer / silent-failure-hunter)
Model: Opus (perf analysis benefits from the larger model)

---

## Skills

The three missing skills from the `2026-05-23-sparrowdb-skills` plan are executed as part of this plan. Their full content is already specced in `docs/superpowers/plans/2026-05-23-sparrowdb-skills.md` Tasks 2–4. This design does not re-specify them — it inherits them.

Priority order (highest first):
1. `docs/skills/debugging.md` — most immediately useful for the current bug-hunting phase
2. `docs/skills/setup.md` — needed for onboarding new agents and contributors
3. `docs/skills/migration.md` — needed when schema changes accompany bug fixes

---

## Hooks

### `scripts/hooks/rust-async-guard.sh`

Trigger: `PostToolUse` on `Edit` and `Write` tool calls.

Behaviour:
1. Read `tool_input.file_path` from stdin JSON
2. If file does not end in `.rs`, exit 0 silently
3. Grep the file for `std::process::Command`
4. If found, check whether the match is inside a `#[cfg(test)]` block or `mod tests` — if so, exit 0 (test-only use is acceptable in blocking contexts per CLAUDE.md)
5. Otherwise print warning and exit 0 (advisory only, never blocks)

Warning format:
```
⚠️  ASYNC GUARD: std::process::Command found in <filepath>
    In async contexts this blocks the Tokio thread pool.
    Replace with tokio::process::Command (see CLAUDE.md §std::process::Command).
```

### `scripts/hooks/cargo-panic-extractor.sh`

Trigger: `PostToolUse` on `Bash` tool calls.

Behaviour:
1. Read `tool_input.command` from stdin JSON; if command does not contain `cargo test` or `cargo bench`, exit 0 silently
2. Read `tool_response` from stdin JSON (the command output)
3. Extract lines matching:
   - `thread '...' panicked at`
   - `GRAPH_ERROR` / `VECTOR_ERROR` / `LMDB`
   - `out of memory` / `OOM` / `Killed`
   - `test result: FAILED`
   - Elapsed time > 30s (slow test warning)
4. If any matches, print a structured block:

```
━━━ PERF/PANIC SUMMARY ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PANICS:    <count>
FAILURES:  <count>
OOM:       yes/no
SLOW (>30s): <count>
━━━ DETAILS ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
<extracted lines>
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### `.claude/settings.json`

New committed project-level file. The existing cargo sweep hook moves here from `settings.local.json` so it applies across all sessions.

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "/Users/franciscobaptista/Development/SparrowDB/scripts/post-build-sweep.sh" },
          { "type": "command", "command": "/Users/franciscobaptista/Development/SparrowDB/scripts/hooks/cargo-panic-extractor.sh" }
        ]
      },
      {
        "matcher": "Edit",
        "hooks": [
          { "type": "command", "command": "/Users/franciscobaptista/Development/SparrowDB/scripts/hooks/rust-async-guard.sh" }
        ]
      },
      {
        "matcher": "Write",
        "hooks": [
          { "type": "command", "command": "/Users/franciscobaptista/Development/SparrowDB/scripts/hooks/rust-async-guard.sh" }
        ]
      }
    ]
  }
}
```

`settings.local.json` retains only the `permissions.allow` list (machine-specific) — the `PostToolUse` Bash hook block currently in `settings.local.json` pointing to `post-build-sweep.sh` must be removed from there once it is present in `settings.json` to avoid double-execution.

---

## CLAUDE.md Updates

### Root `CLAUDE.md` — two new sections

**§ Profiling tools (performance & memory phase)**

| Tool | Install | Purpose |
|------|---------|---------|
| `cargo flamegraph` | `cargo install flamegraph` | CPU flame graph — locate hot functions |
| `cargo +nightly dhat` | nightly toolchain | Heap allocation profile by call site |
| `heaptrack` | system package (Linux) | Live heap growth over time |
| `criterion` | dev-dependency | Reproducible microbenchmarks |
| `sparrow stress` | built-in CLI | End-to-end load test against live instance |
| `SPARROW_SKIP_BM25_ON_WRITE=1` | env var | Isolate BM25 rebuild cost from write latency |
| `POST /rebuild_bm25_index` | HTTP | Trigger and time manual BM25 rebuild |

**§ Agent invocation guide**

| Agent | When to invoke |
|-------|---------------|
| `rust-reviewer` | Before merging any Rust change; runs clippy + safety + SparrowDB invariant checks |
| `rust-build-resolver` | When `cargo build` / `cargo check` fails — workspace-aware diagnosis |
| `silent-failure-hunter` | When a write or query path produces wrong results silently — error propagation audit |
| `sparrow-perf-profiler` | When latency or memory grows unexpectedly — four-phase profiling workflow |

---

### Subdirectory `CLAUDE.md` Files

Each file follows a fixed template: crate purpose (2 lines) → agent pointer table → skill pointer table → local invariants not in root CLAUDE.md → key files. Content is additive to the root, never duplicative.

**`crates/sparrow-core/CLAUDE.md`**
- Purpose: Storage engine, HTTP gateway, HQL compiler, HNSW index, LMDB backend
- Agents: `rust-reviewer` (all changes), `silent-failure-hunter` (error handling paths), `sparrow-perf-profiler` (latency/memory regressions)
- Skills: `docs/skills/debugging.md` (primary reference), `docs/skills/querying.md` (HQL operator reference)
- Local invariants:
  - This crate is `sparrow-core` in Cargo but `sparrow_db` as the lib name — always `use sparrow_db::`
  - Feature flag chain: `lmdb` → `server` → `build + compiler + vectors`; tests need `--features lmdb,server`
  - All mutations route through the single `_writer_worker` in `WorkerPool`; never call `write_txn()` elsewhere
  - LMDB stress tests: `cargo test --package sparrow-core --features lmdb -- --test-threads=1`
- Key files: `src/storage/`, `src/graph_engine/`, `src/compiler/`, `src/helix_gateway/`, `Cargo.toml` lines 80–94 (feature flags)

**`crates/sparrow-container/CLAUDE.md`**
- Purpose: Deployable server binary; wires together sparrow-core with env-var configuration
- Agents: `rust-reviewer`, `rust-build-resolver`
- Skills: `docs/skills/setup.md`
- Local invariants:
  - `src/main.rs` is the canonical source for every `SPARROW_*` env var name — always verify here before documenting elsewhere
  - `SPARROW_RUNTIME_HQL` enables `/__hql_runtime_eval` (confirmed line in main.rs)
  - Any new async code must use `tokio::process::Command`, not `std::process::Command`
- Key files: `src/main.rs` (env vars, startup sequence)

**`crates/sparrow-cli/CLAUDE.md`**
- Purpose: `sparrow` CLI binary — init, build, push, start, stop, migrate, import, stress, etc.
- Agents: `rust-reviewer`, `rust-build-resolver`
- Skills: `docs/skills/setup.md`
- Local invariants:
  - `src/docker.rs` uses `std::process::Command` intentionally — it is called only from **synchronous** contexts. Any refactor making those functions async **must** switch them to `tokio::process::Command`
  - Do not add `async fn` wrappers around the existing docker helper functions without the above change
- Key files: `src/docker.rs` (sync-only docker helpers), `src/commands/` (subcommand handlers)

**`crates/sparrow-macros/CLAUDE.md`**
- Purpose: Procedural macros used by sparrow-core at compile time
- Agents: `rust-reviewer`
- Skills: —
- Local invariants:
  - Proc-macro crate: must not add runtime dependencies; keep `[dependencies]` minimal
  - No `std` features that don't apply in a proc-macro context
  - Changes here affect sparrow-core compile time — test with `cargo build -p sparrow-core --features lmdb,server` after any change

**`sdks/rust/CLAUDE.md`**
- Purpose: `sparrow-sdk` — the public Rust client SDK, publishable to crates.io
- Agents: `rust-reviewer`
- Skills: —
- Local invariants:
  - Licensed Apache-2.0 (not AGPL); this affects what dependencies are permitted
  - **Zero dependency on any `crates/` workspace member** — the SDK must be buildable as a standalone crate
  - Before any publish: verify `cargo publish --dry-run` passes from this directory in isolation

**`tests/hql-tests/CLAUDE.md`**
- Purpose: Integration test harness for HQL queries against a live SparrowDB instance
- Agents: `rust-reviewer`
- Skills: `docs/skills/debugging.md`
- Local invariants:
  - All tests require `--features lmdb,server`
  - LMDB write-conflict tests use `serial_test` — run with `--test-threads=1`
  - Tests that start a live instance should use the `SPARROW_RUNTIME_HQL=1` env var when testing the runtime eval endpoint

---

## Non-goals

- No ECC hooks that require Node.js (keep the hook stack pure bash/Python to match existing `post-build-sweep.sh`)
- No ECC web-focused agents (performance-optimizer, a11y-architect, etc.)
- No ECC skills for languages not used in this repo
- No changes to `sdks/ts/` — the TypeScript SDK is a separate concern

---

## Implementation notes

- Agents are self-contained markdown files; they are invoked via the Claude Code `Agent` tool with `subagent_type` pointing to the file path, or by name if registered
- The `sparrow-perf-profiler` agent uses `Opus` model — perf root-cause analysis benefits from deeper reasoning
- Hook scripts must be executable (`chmod +x`) and must always exit 0 to avoid blocking tool execution
- `post-build-sweep.sh` stays at `scripts/post-build-sweep.sh` (unchanged path); new hook scripts go under `scripts/hooks/` (new subdirectory)
- The `PostToolUse` Bash block in `settings.local.json` referencing `post-build-sweep.sh` must be removed once it appears in `settings.json` to avoid double-execution
- The three missing skills inherit their full content spec from `docs/superpowers/plans/2026-05-23-sparrowdb-skills.md` Tasks 2–4 — the implementation plan for this design should reference that file as the authoritative content source for those files
