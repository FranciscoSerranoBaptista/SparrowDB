# SparrowDB — Full Codebase Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename every HelixDB identifier to SparrowDB — crate names, directories, binary, module paths, struct names, env vars, config paths, and user-facing strings.

**Architecture:** Four sequential tasks, each leaving the workspace in a compilable state. Task 1 handles the crate-level infrastructure (the prerequisite everything else depends on). Tasks 2–3 rename Rust symbols and runtime strings independently. Task 4 cleans up test fixtures. All changes are bulk `sed` + targeted manual edits, verified by `cargo check` or `cargo test` at each stage.

**Tech Stack:** Rust workspace (cargo), `sed`, `find`, `mv`

---

## What we rename vs. what we keep

**Rename (our identifiers):**
- Crate/package names: `helix-*` → `sparrow-*`
- Binary: `helix` → `sparrow`
- Rust module paths in source: `helix_db`, `helix_cli`, `helix_macros`, `helix_metrics`
- Internal modules: `helix_engine` → `sparrow_engine`, `helix_gateway` → `sparrow_gateway`, `helixc` → `sparrowc`
- Public types: `Helix*` → `Sparrow*`
- Macro attribute: `#[helix_node]` → `#[sparrow_node]`
- UI constant: `HELIX_ORANGE` → `SPARROW_ORANGE`
- Env vars: `HELIX_*` → `SPARROW_*`
- Config file: `helix.toml` → `sparrow.toml`
- Data directory: `.helix` → `.sparrow`
- CLI output strings: `"helix init"` → `"sparrow init"` etc.
- `repository` fields in Cargo.toml: replace with your own repo URL

**Do NOT rename (upstream/external):**
- `"Helix Cloud"` — external managed service name
- `https://logs.helix-db.com/v2` — upstream metrics endpoint (disable or replace with your own)
- `helix-db.com` URLs in comments/docs
- `HQL` / `hql-tests` — the query language acronym is not a brand identifier

---

## File Structure

| Directory / File | Change |
|---|---|
| `helix-db/` | → `sparrow-db/` |
| `helix-cli/` | → `sparrow-cli/` |
| `helix-macros/` | → `sparrow-macros/` |
| `helix-container/` | → `sparrow-container/` |
| `helix-ts/` | → `sparrow-ts/` |
| `helix-db/src/helix_engine/` | → `sparrow-db/src/sparrow_engine/` |
| `helix-db/src/helix_gateway/` | → `sparrow-db/src/sparrow_gateway/` |
| `helix-db/src/helixc/` | → `sparrow-db/src/sparrowc/` |
| Root `Cargo.toml` | workspace members updated |
| Every `Cargo.toml` | package names, dep names, lib/bin names |
| All `.rs` files | `use` imports, struct names, env vars, string literals |
| `hql-tests/tests/*/helix.toml` | → `sparrow.toml` (100 files) |

---

## Task 1: Rename Crate Directories, Cargo.toml, and `use` Imports

This is the foundation. After this task, `cargo check` compiles. Tasks 2–4 are blocked on this.

**Files:**
- Rename: `helix-db/` → `sparrow-db/`, `helix-cli/` → `sparrow-cli/`, `helix-macros/` → `sparrow-macros/`, `helix-container/` → `sparrow-container/`, `helix-ts/` → `sparrow-ts/`
- Modify: `Cargo.toml` (root workspace)
- Modify: every `*/Cargo.toml` (package names, lib/bin names, dependency references)
- Modify: all `.rs` files that contain `use helix_*` imports

- [ ] **Step 1: Rename crate directories**

Working directory: `/Users/franciscobaptista/Development/SparrowDB`

```bash
mv helix-db sparrow-db
mv helix-cli sparrow-cli
mv helix-macros sparrow-macros
mv helix-container sparrow-container
mv helix-ts sparrow-ts
```

- [ ] **Step 2: Update root `Cargo.toml` workspace members**

Edit `Cargo.toml` — replace the `members` list:

```toml
[workspace]
members = [
    "sparrow-db",
    "sparrow-container",
    "sparrow-macros",
    "sparrow-cli",
    "hql-tests",
    "metrics",
]
```

- [ ] **Step 3: Bulk-rename all hyphenated crate identifiers in every `Cargo.toml`**

This renames `helix-db`, `helix-cli`, `helix-macros`, `helix-container`, `helix-metrics` everywhere they appear as package names, dependency keys, and `path` values:

```bash
find . -name "Cargo.toml" ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/helix-db/sparrow-db/g' \
    -e 's/helix-cli/sparrow-cli/g' \
    -e 's/helix-macros/sparrow-macros/g' \
    -e 's/helix-container/sparrow-container/g' \
    -e 's/helix-metrics/sparrow-metrics/g' \
  {} +
```

- [ ] **Step 4: Rename `[lib]` and `[[bin]]` `name` fields in Cargo.toml**

The lib name (`helix_cli`) and binary name (`helix`) use underscores/no prefix so the previous sed didn't catch them.

```bash
find . -name "Cargo.toml" ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/name = "helix_cli"/name = "sparrow_cli"/g' \
    -e 's/name = "helix_db"/name = "sparrow_db"/g' \
    -e 's/name = "helix_macros"/name = "sparrow_macros"/g' \
    -e 's/name = "helix_metrics"/name = "sparrow_metrics"/g' \
    -e 's/name = "helix"$/name = "sparrow"/g' \
  {} +
```

Verify the binary name in `sparrow-cli/Cargo.toml` now reads:
```toml
[[bin]]
name = "sparrow"
path = "src/main.rs"
```

- [ ] **Step 5: Update `repository` fields in all Cargo.toml files**

Replace the upstream GitHub repo URL with your own (placeholder — set to your actual repo):

```bash
find . -name "Cargo.toml" ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's|repository = "https://github.com/HelixDB/helix-db"|repository = "https://github.com/YOUR_ORG/SparrowDB"|g' \
  {} +
```

- [ ] **Step 6: Bulk-rename crate-level Rust module names in all `.rs` files**

This updates `use helix_db::`, `use helix_cli::`, `use helix_macros::`, `use helix_metrics::`, and any qualified paths like `helix_db::SomeType`:

```bash
find . -name "*.rs" ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/helix_db::/sparrow_db::/g' \
    -e 's/helix_cli::/sparrow_cli::/g' \
    -e 's/helix_macros::/sparrow_macros::/g' \
    -e 's/helix_metrics::/sparrow_metrics::/g' \
  {} +
```

Then rename bare unqualified `use` imports (e.g. `use helix_db;`, `extern crate helix_macros;`):

```bash
find . -name "*.rs" ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/\bhelix_db\b/sparrow_db/g' \
    -e 's/\bhelix_cli\b/sparrow_cli/g' \
    -e 's/\bhelix_macros\b/sparrow_macros/g' \
    -e 's/\bhelix_metrics\b/sparrow_metrics/g' \
  {} +
```

> **Note:** `\b` is a word boundary. On macOS, use `sed -E` with `[[:<:]]` and `[[:>:]]` if `\b` is unsupported: `sed -E 's/[[:<:]]helix_db[[:>:]]/sparrow_db/g'`. Test the first command on a single file first: `sed -i '' -e 's/\bhelix_db\b/sparrow_db/g' sparrow-db/src/lib.rs` — if it errors, switch to the `-E` form.

- [ ] **Step 7: Verify `cargo check`**

```bash
cargo check --workspace 2>&1 | tail -20
```

Expected: no errors. Warnings about unused imports or renamed items are acceptable. If there are errors, they will be unresolved crate names — fix by checking which Cargo.toml still has the old name.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor: rename helix-* crates to sparrow-* and update all use imports"
```

---

## Task 2: Rename Internal Module Directories and Public Rust Symbols

**Files:**
- Rename (inside `sparrow-db/src/`): `helix_engine/` → `sparrow_engine/`, `helix_gateway/` → `sparrow_gateway/`, `helixc/` → `sparrowc/`
- Modify: `sparrow-db/src/lib.rs` — `mod` declarations for renamed directories
- Modify: all `.rs` files — `Helix*` type names, `helix_engine`/`helix_gateway`/`helixc` module paths, `#[helix_node]` → `#[sparrow_node]`

### Background

After Task 1, the `sparrow_db` crate still has internal modules named `helix_engine`, `helix_gateway`, and `helixc`. This task:
1. Renames those directories
2. Updates `mod` declarations in `lib.rs`
3. Renames all `Helix*` public types to `Sparrow*`
4. Renames the `helix_node` proc-macro attribute to `sparrow_node`

- [ ] **Step 1: Rename internal module directories inside `sparrow-db/src/`**

```bash
mv sparrow-db/src/helix_engine sparrow-db/src/sparrow_engine
mv sparrow-db/src/helix_gateway sparrow-db/src/sparrow_gateway
mv sparrow-db/src/helixc sparrow-db/src/sparrowc
```

- [ ] **Step 2: Update `mod` declarations in `sparrow-db/src/lib.rs`**

Edit `sparrow-db/src/lib.rs`. Replace:
```rust
pub mod helix_engine;
pub mod helix_gateway;
pub mod helixc;
```
with:
```rust
pub mod sparrow_engine;
pub mod sparrow_gateway;
pub mod sparrowc;
```

Run: `grep -n "helix_engine\|helix_gateway\|helixc" sparrow-db/src/lib.rs` to confirm you got them all.

- [ ] **Step 3: Rename internal module paths across all `.rs` files**

```bash
find . -name "*.rs" ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/helix_engine/sparrow_engine/g' \
    -e 's/helix_gateway/sparrow_gateway/g' \
    -e 's/\bhelixc\b/sparrowc/g' \
  {} +
```

- [ ] **Step 4: Rename `Helix*` public types and functions**

```bash
find . -name "*.rs" ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/HelixGraphStorage/SparrowGraphStorage/g' \
    -e 's/HelixGraphEngine/SparrowGraphEngine/g' \
    -e 's/HelixGraphEngineOpts/SparrowGraphEngineOpts/g' \
    -e 's/HelixGateway/SparrowGateway/g' \
    -e 's/HelixRouter/SparrowRouter/g' \
    -e 's/HelixConfig/SparrowConfig/g' \
    -e 's/HelixParser/SparrowParser/g' \
    -e 's/HelixError/SparrowError/g' \
    -e 's/HelixManager/SparrowManager/g' \
  {} +
```

Then catch any remaining `Helix` prefix in type names (scan first, then apply):

```bash
# Scan for anything missed
grep -rn "struct Helix\|enum Helix\|trait Helix\|impl Helix\|fn helix_" \
  sparrow-db/src sparrow-cli/src sparrow-container/src sparrow-macros/src \
  ! -path "*/target/*" | grep -v "Helix Cloud"
```

For each hit, apply the rename manually if it wasn't caught above.

- [ ] **Step 5: Rename `helix_node` proc-macro to `sparrow_node`**

Edit `sparrow-macros/src/lib.rs`. Find:
```rust
pub fn helix_node(_attr: TokenStream, input: TokenStream) -> TokenStream {
```
Replace with:
```rust
pub fn sparrow_node(_attr: TokenStream, input: TokenStream) -> TokenStream {
```

Then update all call sites across the codebase:
```bash
find . -name "*.rs" ! -path "*/target/*" \
  -exec sed -i '' 's/helix_node/sparrow_node/g' {} +
```

- [ ] **Step 6: Rename `HELIX_ORANGE` UI color constant**

Edit `sparrow-cli/src/commands/logs/tui.rs`. Replace all occurrences:
```bash
sed -i '' 's/HELIX_ORANGE/SPARROW_ORANGE/g' sparrow-cli/src/commands/logs/tui.rs
```

- [ ] **Step 7: Verify `cargo check`**

```bash
cargo check --workspace 2>&1 | tail -20
```

Expected: no errors. If you see `unresolved module` errors, check that all three directory renames in Step 1 were applied and `lib.rs` `mod` declarations match.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor: rename internal modules (sparrow_engine, sparrow_gateway, sparrowc) and Helix* types to Sparrow*"
```

---

## Task 3: Rename Env Vars, Config Paths, and User-Facing Strings

**Files:**
- Modify: all `.rs` and `.sh` files containing `HELIX_*`, `"helix.toml"`, `".helix"`, `"helix init"` etc.
- Modify: `sparrow-container/src/main.rs` — `HELIX_DATA_DIR`, `HELIX_PORT`

### Background

This task has no compilation impact — string literals compile regardless of value. However, getting them right is critical for the CLI to work correctly at runtime. The changes are:
- 14 distinct `HELIX_*` env vars → `SPARROW_*`
- `"helix.toml"` config file name → `"sparrow.toml"`
- `".helix"` home directory → `".sparrow"`
- User-facing CLI command strings (`"helix init"`, `"helix build"` etc.) → `"sparrow ..."`
- `"HelixDB"` product name in output strings → `"SparrowDB"`
- Banner text `"> HELIX DB"` → `"> SPARROW DB"`

Do NOT rename `"Helix Cloud"` — that is an external managed service.

- [ ] **Step 1: Rename all `HELIX_*` environment variable names**

```bash
find . \( -name "*.rs" -o -name "*.sh" -o -name "*.toml" -o -name "*.yml" \) \
  ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/HELIX_DATA_DIR/SPARROW_DATA_DIR/g' \
    -e 's/HELIX_PORT/SPARROW_PORT/g' \
    -e 's/HELIX_API_KEY/SPARROW_API_KEY/g' \
    -e 's/HELIX_CLUSTER_ID/SPARROW_CLUSTER_ID/g' \
    -e 's/HELIX_CORES_OVERRIDE/SPARROW_CORES_OVERRIDE/g' \
    -e 's/HELIX_INSTANCE/SPARROW_INSTANCE/g' \
    -e 's/HELIX_PROJECT/SPARROW_PROJECT/g' \
    -e 's/HELIX_HOME/SPARROW_HOME/g' \
    -e 's/HELIX_CACHE_DIR/SPARROW_CACHE_DIR/g' \
    -e 's/HELIX_HOST/SPARROW_HOST/g' \
    -e 's/HELIX_CLOUD_URL/SPARROW_CLOUD_URL/g' \
    -e 's/HELIX_USER_ID/SPARROW_USER_ID/g' \
    -e 's/HELIX_METRICS_THRESHOLD_BATCHES/SPARROW_METRICS_THRESHOLD_BATCHES/g' \
    -e 's/HELIX_RUNTIME_HQL/SPARROW_RUNTIME_HQL/g' \
  {} +
```

- [ ] **Step 2: Rename config file name and home directory path**

```bash
find . \( -name "*.rs" -o -name "*.sh" \) ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/"helix\.toml"/"sparrow.toml"/g' \
    -e "s/\"helix.toml\"/\"sparrow.toml\"/g" \
    -e 's/\.join("helix\.toml")/.join("sparrow.toml")/g' \
    -e 's/\.join("\.helix")/.join(".sparrow")/g' \
    -e 's|"\.helix/|".sparrow/|g' \
    -e 's/\\"helix\.toml\\"/\\"sparrow.toml\\"/g' \
  {} +
```

Then scan for any remaining `helix.toml` references not caught:
```bash
grep -rn '"helix.toml"\|helix\.toml\|"\.helix"' . --include="*.rs" ! -path "*/target/*"
```
Fix any remaining occurrences manually.

- [ ] **Step 3: Rename user-facing CLI output strings**

```bash
find . -name "*.rs" ! -path "*/target/*" \
  -exec sed -i '' \
    -e "s/'helix /'sparrow /g" \
    -e 's/"helix init"/"sparrow init"/g' \
    -e 's/"helix build"/"sparrow build"/g' \
    -e 's/"helix push"/"sparrow push"/g' \
    -e 's/"helix update"/"sparrow update"/g' \
    -e 's/"helix status"/"sparrow status"/g' \
    -e 's/"helix data"/"sparrow data"/g' \
    -e 's/Run '\''helix /Run '\''sparrow /g' \
    -e "s/run 'helix /run 'sparrow /g" \
  {} +
```

Then rename the product name in output strings — but SKIP "Helix Cloud":
```bash
find . -name "*.rs" ! -path "*/target/*" \
  -exec sed -i '' \
    -e 's/HelixDB/SparrowDB/g' \
    -e 's/Helix DB CLI/SparrowDB CLI/g' \
    -e 's/Helix DB/SparrowDB/g' \
  {} +
```

- [ ] **Step 4: Update the CLI welcome banner**

Edit `sparrow-cli/src/main.rs`. Find the banner text:
```rust
if let Ok(banner) = Banner::new("> HELIX DB") {
```
Replace with:
```rust
if let Ok(banner) = Banner::new("> SPARROW DB") {
```

- [ ] **Step 5: Update `helix-container` binary invocation in `run.rs`**

Edit `sparrow-cli/src/commands/run.rs`. The binary is named `helix-container` — change it to `sparrow-container`:

```bash
sed -i '' 's/"helix-container"/"sparrow-container"/g' sparrow-cli/src/commands/run.rs
```

Also update the docker.rs Dockerfile template for the container binary name:
```bash
grep -n "helix-container" sparrow-cli/src/docker.rs
```
For each occurrence, update `helix-container` → `sparrow-container`.

- [ ] **Step 6: Verify `cargo check`**

```bash
cargo check --workspace 2>&1 | tail -20
```

Expected: no errors.

- [ ] **Step 7: Disable upstream metrics sending in `metrics/src/lib.rs`**

The constant `METRICS_URL` points to `https://logs.helix-db.com/v2` — HelixDB's telemetry server. Disable the send by making the batch-send function a no-op.

Find the function that calls `.post(METRICS_URL)` (around line 280). It will look something like:

```rust
async fn send_batch(&self, batch: Vec<MetricEvent>) -> Result<(), ...> {
    // ... builds request ...
    let response = self.client
        .post(METRICS_URL)
        ...
```

Replace the entire function body with an early return:

```rust
async fn send_batch(&self, _batch: Vec<MetricEvent>) -> Result<(), ...> {
    // Telemetry disabled
    return Ok(());
}
```

Keep the `METRICS_URL` constant and all other types — just stub out the send so nothing is transmitted.

Verify the function compiles:
```bash
cargo check -p sparrow-metrics 2>&1 | tail -10
```

Expected: no errors (there will be `unused variable` warnings — acceptable).

- [ ] **Step 8: Scan for any missed helix references in Rust source**

```bash
grep -rn "\bhelix\b" . --include="*.rs" ! -path "*/target/*" \
  | grep -v "Helix Cloud\|helix-db\.com\|HelixDB/helix" \
  | head -30
```

Fix any remaining occurrences that are our identifiers (not upstream references).

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: rename HELIX_* env vars to SPARROW_*, helix.toml to sparrow.toml, .helix to .sparrow, update CLI strings, and disable upstream telemetry"
```

---

## Task 4: Rename Test Fixtures and Update Test Assertions

**Files:**
- Rename: 100× `hql-tests/tests/*/helix.toml` → `sparrow.toml`
- Modify: `sparrow-cli/src/tests/docker_tests.rs` — hardcoded `"helix.toml"`, `".helix"`, `HELIX_DATA_DIR`
- Modify: `sparrow-cli/src/tests/compile_tests.rs` — `helix.toml` fixture references
- Modify: `sparrow-cli/src/tests/test_utils.rs` — `SPARROW_CACHE_DIR`, `SPARROW_HOME` (now renamed but tests may still reference old names)
- Modify: `hql-tests/src/main.rs` — any hardcoded `"helix.toml"` lookup logic

- [ ] **Step 1: Rename the 100 `helix.toml` fixture files in `hql-tests`**

```bash
find hql-tests/tests -name "helix.toml" -exec sh -c \
  'mv "$1" "$(dirname "$1")/sparrow.toml"' _ {} \;
```

Verify the rename:
```bash
find hql-tests/tests -name "helix.toml" | wc -l   # should print 0
find hql-tests/tests -name "sparrow.toml" | wc -l  # should print 100
```

- [ ] **Step 2: Update `hql-tests/src/main.rs` to look for `sparrow.toml`**

```bash
grep -n "helix.toml\|helix_toml\|\"\.helix\"" hql-tests/src/main.rs
```

For each occurrence, apply the rename:
```bash
sed -i '' \
  -e 's/helix\.toml/sparrow.toml/g' \
  -e 's/"\.helix"/"\.sparrow"/g' \
  hql-tests/src/main.rs
```

- [ ] **Step 3: Update test fixture references in `sparrow-cli` tests**

```bash
find sparrow-cli/src/tests -name "*.rs" \
  -exec sed -i '' \
    -e 's/"helix\.toml"/"sparrow.toml"/g' \
    -e 's/\.join("helix\.toml")/.join("sparrow.toml")/g' \
    -e 's/\.join("\.helix")/.join(".sparrow")/g' \
    -e 's/HELIX_CACHE_DIR/SPARROW_CACHE_DIR/g' \
    -e 's/HELIX_HOME/SPARROW_HOME/g' \
    -e 's/HELIX_DATA_DIR/SPARROW_DATA_DIR/g' \
  {} +
```

> **Note:** If Task 3's sed already handled these, this step will be a no-op — run it anyway to be safe.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test --workspace 2>&1 | tail -30
```

Expected: 1 pre-existing failure (`test_init_preserves_existing_scaffold_files_non_interactive`), no new failures. If tests fail with `"helix.toml not found"` or similar, scan for remaining hardcoded references.

- [ ] **Step 5: Scan for any remaining `helix` references across the whole workspace**

```bash
grep -rn "helix" . \
  --include="*.rs" --include="*.toml" --include="*.sh" \
  ! -path "*/target/*" ! -path "*/.git/*" \
  | grep -iv "Helix Cloud\|helix-db\.com\|HelixDB/helix\|sparrow" \
  | head -40
```

Review each hit. Fix any that are our identifiers, not upstream references.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: rename helix.toml test fixtures to sparrow.toml and update test assertions"
```

---

## Self-Review

**Spec coverage:**

| Rename category | Task | Status |
|---|---|---|
| Crate directories (`helix-*` → `sparrow-*`) | 1 | ✓ |
| Cargo.toml package/dep names | 1 | ✓ |
| Binary name (`helix` → `sparrow`) | 1 | ✓ |
| `use helix_*` Rust imports | 1 | ✓ |
| Internal module dirs (`helix_engine` etc.) | 2 | ✓ |
| `Helix*` public type names | 2 | ✓ |
| `#[helix_node]` macro → `#[sparrow_node]` | 2 | ✓ |
| `HELIX_ORANGE` color constant | 2 | ✓ |
| `HELIX_*` env vars | 3 | ✓ |
| `helix.toml` config filename | 3 | ✓ |
| `.helix` home dir | 3 | ✓ |
| User-facing CLI strings | 3 | ✓ |
| Banner text | 3 | ✓ |
| `helix-container` binary name in code | 3 | ✓ |
| hql-tests fixture files | 4 | ✓ |
| Test assertions | 4 | ✓ |
| Repository URLs | 1 | ✓ (placeholder) |

**What this plan does NOT change (by design):**
- `"Helix Cloud"` — external service
- `https://logs.helix-db.com` — upstream metrics URL (owner can decide separately)
- `HQL` / `hql-tests` — query language acronym, not brand
- `helix-db.com` domain in comments/docs
