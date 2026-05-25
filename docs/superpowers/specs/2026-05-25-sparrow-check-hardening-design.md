# sparrow check Hardening Design

**Date:** 2026-05-25  
**Status:** Approved  
**Scope:** `sparrow check` validation pipeline, version stamp, `sparrow doctor` command

---

## Problem

The local development loop has four distinct friction layers:

1. **Codegen regression loop** — `sparrow check` can generate invalid Rust (`queries.rs`). The user doesn't discover this until `docker build` fails, 10+ minutes later. The broken file is written to disk and travels forward silently.

2. **CLI binary staleness** — the installed `~/.cargo/bin/sparrow` binary is built once from SparrowDB source. When SparrowDB is patched (e.g. WHERE optimizer fix), the binary doesn't automatically update. Nothing warns that the binary is stale.

3. **Coupled artefacts** — `sparrow-container/src/queries.rs` is written by `sparrow check`, copied into a Docker build context, and compiled inside the image. Three locations for one logical artefact. When one is updated, it's unclear which version is live.

4. **No authoritative health check** — each step in the pipeline (`cargo check`, `sparrow check`, `docker build`, container start, auth) can silently succeed while the next silently breaks.

**Invariant to enforce:** `queries.rs` on disk always reflects a schema that compiles. The user iterates on HQL without needing to undo anything.

---

## Design

### 1. `sparrow check` validation pipeline

Four ordered stages. Failure at any stage leaves the existing `queries.rs` intact and surfaces an attributed error. The atomic rename only happens if all four stages pass.

```
sparrow check
│
├── Stage 0: HQL analysis (existing)
│   Runs the ariadne compiler. HQL source location attributed on failure.
│   On pass: generated AST held in memory — nothing written to disk yet.
│
├── Stage 1: Generator self-assertions (in-process, ~0ms)
│   Walks the generated AST before serialisation. Checks known-bad structural patterns:
│     • NFromIndex key arguments must be GenRef::Ref or GenRef::Std (no .clone() suffix)
│     • AddN/UpsertN property maps must not contain Unknown GeneratedValues
│     • (extensible — new assertions added per discovered bug class)
│   Failure → "known codegen bug: Query 'X', field 'Y' — [assertion message].
│              This is a SparrowDB bug. Please report with your .hx files."
│   No cargo check penalty paid. Exits immediately with precise message.
│
├── Stage 2: cargo check (subprocess, 30–60s cold / incremental thereafter)
│   Serialises generated code to a temp file at <project>/.sparrow/validate/queries.rs.
│   Invokes: cargo check --message-format=json
│   in the hermetic cached workspace, pinned to the binary's embedded commit hash.
│   Parses JSON error output. On failure: locates the enclosing query marker comment
│   (see Section 3) → emits:
│     "codegen bug in query 'UpsertZettelNote' (instruments.hx:42):
│      Rust error: mismatched types — expected `&_`, found `String`
│      This is a SparrowDB bug. Run --debug-codegen to inspect the output.
│      Please report at: https://github.com/YOUR_ORG/SparrowDB/issues"
│   If no marker found: emits the Rust error verbatim with "likely a codegen bug" prefix.
│
└── Stage 3: Atomic rename (on all-pass)
    Moves validated temp file to queries.rs (POSIX rename — atomic on Linux/macOS).
    Writes version stamp header (Section 2).
    Reports success with query count.
```

**`--debug-codegen` flag:** skips Stages 1 and 2. Writes the generated output directly to `queries.rs` (or an explicit path). The file is prepended with:
```rust
// ⚠ DEBUG — UNVALIDATED OUTPUT — do not use in production
// Generated with --debug-codegen. Run `sparrow check` for validated output.
```

#### Layering rationale

Stage 1 (self-assertions) runs first at zero cost. If it fires, the user gets a precise "known bug class" message without paying the `cargo check` penalty. Stage 2 is the completeness guarantee — it catches novel codegen bugs that Stage 1 hasn't enumerated. The 30–60s cost is only paid when Stage 1 doesn't catch it, which is exactly when it's needed.

Stage 1's coverage degrades over time as the generator evolves; Stage 2's does not. Stage 1 belongs as a fast-path triage layer in front of Stage 2, not as an alternative to it.

---

### 2. Version stamp

**Embedding in the binary**

A `build.rs` in `sparrow-cli` runs at compile time:

```rust
// crates/sparrow-cli/build.rs
fn main() {
    let hash = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=SPARROW_BUILD_HASH={hash}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
```

The binary reads this at runtime via `env!("SPARROW_BUILD_HASH")`. In Docker builds (`.git` present at image-build time), the hash is baked in. If `.git` is absent, the value is `"unknown"` — comparisons are skipped silently.

The dirty flag (`-dirty`) is required: a clean commit hash can still mean "source was modified but not committed." `abc1234-dirty` vs `abc1234` catches "I patched source and rebuilt but forgot to commit."

**The stamp in `queries.rs`**

The first three lines of every validated `queries.rs`:

```rust
// generated by sparrow abc1234-dirty
// sparrow-core workspace: abc1234-dirty
// Do not edit — regenerate with `sparrow check`
```

Both lines use the same hash since `sparrow check` pins the workspace to the binary's commit before running Stage 2.

**Comparison at `sparrow check` time**

Before Stage 0, `sparrow check` reads the existing header (if any) and compares the `generated by sparrow` hash against `env!("SPARROW_BUILD_HASH")`:

- **Match:** proceed normally.
- **Mismatch:** emit a warning and continue: `"queries.rs was generated by sparrow abc1234 but you are running abc5678-dirty. Regenerating…"`
- **Either side `"unknown"`:** skip comparison silently.

---

### 3. HQL-attributed error messages

The code generator emits a one-line marker at the top of each generated query function body. The marker carries the HQL query name and source location from the `Loc` struct already tracked by the analyzer:

```rust
// sparrow:query=UpsertZettelNote source=instruments.hx:42
fn post_upsert_zettel_note(input: HandlerInput) -> ... {
```

When `cargo check --message-format=json` reports an error at line N, `sparrow check` scans backward from line N to the nearest `sparrow:query=` comment and extracts the query name and source location. No new span tracking is required — the generator propagates the existing `Loc` through to the comment.

If the backward scan finds no marker (generated preamble, imports, schema registration), the fallback message is used.

---

### 4. Hermetic workspace pinning

The `cargo check` in Stage 2 must run against exactly the sparrow-core version the binary was built from. Using a different version would give a false green.

Pinning mechanism:

1. `sparrow check` reads `SPARROW_BUILD_HASH` from the binary.
2. Resolves the cached workspace path (same root as `ensure_sparrow_repo_cached()`).
3. Checks the workspace's current git HEAD against the binary hash. If they differ, runs `git checkout <hash>` in the cached repo before Stage 2.
4. If the hash is `"unknown"`: uses whatever is currently cached, emits a warning that validation may not be hermetic.
5. `cargo check` is invoked with `--manifest-path <cached-workspace>/crates/sparrow-container/Cargo.toml` — the same `Cargo.toml` and `Cargo.lock` that `docker build` uses. No separate manifest to maintain.

The temp `queries.rs` is copied into `<cached-workspace>/crates/sparrow-container/src/queries.rs` before `cargo check`. After the check completes (pass or fail), the workspace file is restored to its previous state.

---

### 5. `sparrow doctor`

A standalone pre-flight command. No `cargo` invocations; runs in under 2 seconds. Output is a human-readable checklist; `--json` emits a machine-readable object for CI.

**Output format:**

```
$ sparrow doctor

  ✓ CLI: sparrow v3.0.0 (abc1234-dirty)
  ✓ queries.rs: in sync with CLI (abc1234-dirty)
  ✓ Cached workspace: present and pinned to abc1234
  ✗ Docker: daemon not running
      → Start Docker Desktop or Podman before running `sparrow push`
  ✓ Instance 'dev': running on :6969 — 42 nodes, 0 vectors
  ✗ Instance 'prod': not found
      → Run `sparrow push prod` to deploy

  2 issues found. Fix the ✗ items above before deploying.
```

**Checks:**

| Check | Source | Failure blocks |
|-------|--------|---------------|
| CLI version | `env!("SPARROW_BUILD_HASH")` + `CARGO_PKG_VERSION` | Nothing — informational |
| `queries.rs` in sync | Header hash vs binary hash | `sparrow push` (stale output) |
| Cached workspace pinned | Workspace HEAD vs binary hash | Stage 2 validation |
| Docker running | `docker info` exit code | `sparrow push` |
| Instance health (per configured instance) | `GET /diagnostics` on configured port | Nothing — informational |

Checks are run in parallel where independent (Docker + instance health). Items blocked by an earlier failure (Docker not running → can't check instance) are shown as `? skipped: Docker not available`.

Exit code 0 if no blocking failures; exit code 1 if any blocking check fails.

---

## Non-goals

- No `sparrow watch` or hot-reload — `sparrow check` remains a deliberate gate, not a file-watcher daemon.
- No change to `sparrow build` or `sparrow push` pipelines — they remain independent. `sparrow doctor` surfaces issues before those commands are run.
- No automated `cargo install` of a fresh binary — `sparrow doctor` warns about staleness but does not self-update.

---

## Files created or modified

| File | Change |
|------|--------|
| `crates/sparrow-cli/build.rs` | New — embeds `SPARROW_BUILD_HASH` |
| `crates/sparrow-cli/src/commands/check.rs` | Modified — atomic swap, 4-stage pipeline, version stamp |
| `crates/sparrow-cli/src/commands/doctor.rs` | New — pre-flight checklist command |
| `crates/sparrow-cli/src/commands/mod.rs` | Modified — register `doctor` subcommand |
| `crates/sparrow-core/src/sparrowc/generator/` | Modified — emit `sparrow:query=` markers; add Stage 1 self-assertions |
