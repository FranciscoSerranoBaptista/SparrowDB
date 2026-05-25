# Runtime Settings API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `GET /settings` and `POST /settings` endpoints to SparrowDB so operators can inspect and toggle hot-swappable settings at runtime without restarting the container.

**Architecture:** A new `RuntimeSettings` struct holds an `Arc<AtomicBool>` for `skip_bm25_on_write` (mutable at runtime) and a `usize` for `worker_threads` (immutable, observability only). The same Arc is shared between `SparrowGraphStorage` (reads it on every write) and `AppState` (the settings endpoint writes it). Source tracking (`"env"` / `"default"` / `"runtime"`) is stored alongside each mutable setting. `POST /settings` changes are ephemeral — env vars remain the source of truth at restart.

**Tech Stack:** Rust, Tokio, Axum, `std::sync::atomic::{AtomicBool, Ordering}`, `std::sync::{Arc, Mutex}`, `serde_json`.

---

## File Map

| Action | File |
|--------|------|
| Create | `crates/sparrow-core/src/sparrow_gateway/settings.rs` |
| Modify | `crates/sparrow-core/src/sparrow_gateway/mod.rs` |
| Create | `crates/sparrow-core/src/sparrow_gateway/builtin/settings_handler.rs` |
| Modify | `crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs` |
| Modify | `crates/sparrow-core/src/sparrow_gateway/gateway.rs` |
| Modify | `crates/sparrow-core/src/sparrow_engine/traversal_core/mod.rs` |
| Modify | `crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs` |
| Modify (×4) | `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/drop.rs` |
| Modify (×4) | `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/update.rs` |
| Modify (×4) | `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/upsert.rs` (2 sites) |
| Modify (×4) | `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_n.rs` (2 sites) |
| Modify | `crates/sparrow-container/src/main.rs` |

---

## Codebase Context

**`SparrowGraphStorage` (storage_core/mod.rs):**
- `pub skip_bm25_writes: bool` at line 88 — to be changed to `Arc<AtomicBool>`
- Initialized at line 248-251 from `SPARROW_SKIP_BM25_ON_WRITE` env var
- Used in 6 places across 4 files as `!storage.skip_bm25_writes` (to become `.load(Ordering::Relaxed)`)

**`SparrowGraphEngineOpts` (traversal_core/mod.rs line 29):**
- Currently: `pub path: String`, `pub config: Config`, `pub version_info: VersionInfo`
- Will add: `pub skip_bm25_on_write: Option<Arc<AtomicBool>>`

**`AppState` (gateway.rs):**
- Currently: `pub worker_pool`, `pub schema_json`, `pub cluster_id`, `pub token_store`
- Will add: `pub settings: Arc<RuntimeSettings>`

**`SparrowGateway::new()` (gateway.rs line 59):**
- Currently takes: `address, graph_access, workers_per_core, routes, mcp_routes, write_routes, opts`
- Will add: `settings: Arc<RuntimeSettings>`

**Auth pattern (token_mgmt.rs):**
- `extract_verified_admin(state, headers)` — returns `Err(Response)` if not Admin
- Uses `headers.get("x-api-key")` to extract token
- `state.token_store.is_auth_required()` — if false, auth is disabled (bootstrap mode)
- For read-access: simpler check — verify token if auth is required, allow any role

**`main.rs` (sparrow-container/src/main.rs):**
- Line 122: constructs `SparrowGraphEngineOpts { path, config, version_info }`
- Line 185: calls `SparrowGateway::new(address, graph, workers_per_core, ...)`
- `SPARROW_SKIP_BM25_ON_WRITE` log message at line 75

**Handler registration pattern:**
- Builtin axum handlers (like `/tokens`) are added manually in `gateway.rs` `run()` function
- Registered with `axum_app = axum_app.route("/settings", get(...).post(...))`
- Under `#[cfg(feature = "lmdb")]` since they require the token store for auth

**Worker threads:**
- Parsed from `SPARROW_WORKER_THREADS` at runtime in `gateway.rs` `run()` (line 146-163)
- Default: `min(4 × cores, 64)`, enforced even
- Not stored in opts — computed locally in `run()`; thus RuntimeSettings reads it independently at startup

---

### Task 1: `RuntimeSettings` Struct

**Files:**
- Create: `crates/sparrow-core/src/sparrow_gateway/settings.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/mod.rs`

`RuntimeSettings` is the central settings store. It holds the `Arc<AtomicBool>` for `skip_bm25_on_write` (shared with `SparrowGraphStorage`), plus source tracking so `GET /settings` can report `"env"`, `"default"`, or `"runtime"`.

- [ ] **Step 1: Write the failing tests**

Create `crates/sparrow-core/src/sparrow_gateway/settings.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_reads_skip_bm25_default() {
        // Without env var set, should default to false
        std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE");
        let s = RuntimeSettings::from_env();
        assert!(!s.skip_bm25_on_write.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "default");
    }

    #[test]
    fn from_env_reads_skip_bm25_from_env() {
        std::env::set_var("SPARROW_SKIP_BM25_ON_WRITE", "1");
        let s = RuntimeSettings::from_env();
        assert!(s.skip_bm25_on_write.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "env");
        std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE");
    }

    #[test]
    fn set_skip_bm25_changes_value_and_source() {
        let s = RuntimeSettings::from_env();
        s.set_skip_bm25_on_write(true);
        assert!(s.skip_bm25_on_write.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "runtime");
    }

    #[test]
    fn worker_threads_comes_from_env_or_default() {
        std::env::remove_var("SPARROW_WORKER_THREADS");
        let s = RuntimeSettings::from_env();
        // Default: min(4 * cores, 64), ≥ 2 and even
        assert!(s.worker_threads >= 2);
        assert_eq!(s.worker_threads % 2, 0);
    }
}
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cargo test --package sparrow-core --features lmdb -- settings::tests
```

Expected: FAIL — module doesn't exist yet.

- [ ] **Step 3: Implement `settings.rs`**

```rust
// crates/sparrow-core/src/sparrow_gateway/settings.rs

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How a setting's current value was established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingSource {
    /// Value came from an environment variable at startup.
    Env,
    /// No env var was set; value is the compiled-in default.
    Default,
    /// Value was changed via `POST /settings` this session (ephemeral).
    Runtime,
}

impl SettingSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SettingSource::Env => "env",
            SettingSource::Default => "default",
            SettingSource::Runtime => "runtime",
        }
    }
}

/// Hot-swappable operational settings for a running SparrowDB instance.
///
/// Mutable fields use `Arc<AtomicBool>` so changes are immediately visible
/// to all code paths that hold a clone of the Arc (including storage_core).
///
/// Immutable fields (like `worker_threads`) are read-only — included for
/// observability via `GET /settings` but rejected by `POST /settings`.
///
/// All changes via `POST /settings` are ephemeral. The next restart restores
/// env var values.
#[derive(Debug, Clone)]
pub struct RuntimeSettings {
    // ── mutable ───────────────────────────────────────────────────────────
    /// Skip BM25 index rebuild on writes. Equivalent to `SPARROW_SKIP_BM25_ON_WRITE=1`.
    pub skip_bm25_on_write: Arc<AtomicBool>,
    pub skip_bm25_source: Arc<Mutex<SettingSource>>,

    // ── immutable (observability only) ────────────────────────────────────
    /// Number of worker threads. Set at startup, not changeable at runtime.
    pub worker_threads: usize,
}

impl RuntimeSettings {
    /// Construct from environment variables, applying defaults where not set.
    pub fn from_env() -> Self {
        let (skip_value, skip_source) =
            match std::env::var("SPARROW_SKIP_BM25_ON_WRITE").as_deref() {
                Ok("1") | Ok("true") | Ok("True") | Ok("TRUE") => (true, SettingSource::Env),
                Ok(_) => (false, SettingSource::Env),
                Err(_) => (false, SettingSource::Default),
            };

        let worker_threads = std::env::var("SPARROW_WORKER_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map(|n| {
                let n = n.max(2);
                if n % 2 == 0 { n } else { n + 1 }
            })
            .unwrap_or_else(|| {
                let cores = num_cpus::get().max(1);
                let n = (cores * 4).min(64).max(2);
                if n % 2 == 0 { n } else { n + 1 }
            });

        RuntimeSettings {
            skip_bm25_on_write: Arc::new(AtomicBool::new(skip_value)),
            skip_bm25_source: Arc::new(Mutex::new(skip_source)),
            worker_threads,
        }
    }

    /// Toggle `skip_bm25_on_write` at runtime and mark source as "runtime".
    pub fn set_skip_bm25_on_write(&self, value: bool) {
        self.skip_bm25_on_write.store(value, Ordering::Relaxed);
        if let Ok(mut src) = self.skip_bm25_source.lock() {
            *src = SettingSource::Runtime;
        }
    }

    /// Serialize all settings to a JSON string for `GET /settings` response.
    pub fn to_json(&self) -> String {
        let skip_val = self.skip_bm25_on_write.load(Ordering::Relaxed);
        let skip_src = self
            .skip_bm25_source
            .lock()
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let workers = self.worker_threads;

        format!(
            r#"{{"settings":{{"skip_bm25_on_write":{{"value":{skip_val},"source":"{skip_src}","mutable":true}},"worker_threads":{{"value":{workers},"source":"env","mutable":false}}}}}}"#,
        )
    }
}

#[cfg(test)]
mod tests {
    // (tests from Step 1 go here)
    use super::*;

    #[test]
    fn from_env_reads_skip_bm25_default() {
        std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE");
        let s = RuntimeSettings::from_env();
        assert!(!s.skip_bm25_on_write.load(Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "default");
    }

    #[test]
    fn from_env_reads_skip_bm25_from_env() {
        std::env::set_var("SPARROW_SKIP_BM25_ON_WRITE", "1");
        let s = RuntimeSettings::from_env();
        assert!(s.skip_bm25_on_write.load(Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "env");
        std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE");
    }

    #[test]
    fn set_skip_bm25_changes_value_and_source() {
        std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE");
        let s = RuntimeSettings::from_env();
        s.set_skip_bm25_on_write(true);
        assert!(s.skip_bm25_on_write.load(Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "runtime");
    }

    #[test]
    fn worker_threads_is_even_and_at_least_2() {
        std::env::remove_var("SPARROW_WORKER_THREADS");
        let s = RuntimeSettings::from_env();
        assert!(s.worker_threads >= 2);
        assert_eq!(s.worker_threads % 2, 0);
    }

    #[test]
    fn to_json_includes_both_settings() {
        std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE");
        let s = RuntimeSettings::from_env();
        let json = s.to_json();
        assert!(json.contains("skip_bm25_on_write"));
        assert!(json.contains("worker_threads"));
        assert!(json.contains("\"mutable\":true"));
        assert!(json.contains("\"mutable\":false"));
    }
}
```

- [ ] **Step 4: Register the module in `mod.rs`**

In `crates/sparrow-core/src/sparrow_gateway/mod.rs`, add:

```rust
pub mod settings;
```

- [ ] **Step 5: Run the tests to confirm they pass**

```bash
cargo test --package sparrow-core --features lmdb -- settings::tests
```

Expected: All 5 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/sparrow-core/src/sparrow_gateway/settings.rs \
        crates/sparrow-core/src/sparrow_gateway/mod.rs
git commit -m "feat(gateway): add RuntimeSettings struct with AtomicBool for hot-swappable skip_bm25_on_write"
```

---

### Task 2: Make `skip_bm25_writes` Hot-Swappable in Storage

**Files:**
- Modify: `crates/sparrow-core/src/sparrow_engine/traversal_core/mod.rs`
- Modify: `crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs`
- Modify: `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/drop.rs`
- Modify: `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/update.rs`
- Modify: `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/upsert.rs`
- Modify: `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_n.rs`

Change `skip_bm25_writes: bool` to `skip_bm25_writes: Arc<AtomicBool>` throughout the storage layer. The 6 use sites all use the same pattern `!storage.skip_bm25_writes` which becomes `!storage.skip_bm25_writes.load(Ordering::Relaxed)`.

- [ ] **Step 1: Write the failing test**

In `storage_core/mod.rs` (in the `#[cfg(feature = "lmdb")]` cfg block), look for or add a `#[cfg(test)]` module at the bottom and add:

```rust
#[cfg(test)]
mod skip_bm25_tests {
    // This is a compile-time test: the field type must be Arc<AtomicBool>.
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn assert_arc_atomic_bool(_: &Arc<AtomicBool>) {}

    // If skip_bm25_writes is still a plain bool, the compiler will reject this
    // function (wrong type). No runtime assertion needed.
    fn check_field_type(s: &SparrowGraphStorage) {
        assert_arc_atomic_bool(&s.skip_bm25_writes);
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test --package sparrow-core --features lmdb -- skip_bm25_tests 2>&1 | head -20
```

Expected: FAIL — type mismatch (`bool` is not `Arc<AtomicBool>`).

- [ ] **Step 3: Add `skip_bm25_on_write` to `SparrowGraphEngineOpts`**

In `crates/sparrow-core/src/sparrow_engine/traversal_core/mod.rs`, update `SparrowGraphEngineOpts`:

```rust
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Default, Clone)]
pub struct SparrowGraphEngineOpts {
    pub path: String,
    pub config: Config,
    pub version_info: VersionInfo,
    /// Shared atomic controlling BM25 skip behavior.
    /// When None, the storage layer reads SPARROW_SKIP_BM25_ON_WRITE from env.
    pub skip_bm25_on_write: Option<Arc<AtomicBool>>,
}
```

Update `SparrowGraphEngine::new()` to pass it through to storage:

```rust
pub fn new(opts: SparrowGraphEngineOpts) -> Result<SparrowGraphEngine, GraphError> {
    let should_use_mcp = opts.config.mcp;
    let storage = match SparrowGraphStorage::new(
        opts.path.as_str(),
        opts.config,
        opts.version_info,
        opts.skip_bm25_on_write,  // ← new parameter
    ) {
        Ok(db) => Arc::new(db),
        Err(err) => return Err(err),
    };
    // rest unchanged
```

- [ ] **Step 4: Update `SparrowGraphStorage::new()` signature and field**

In `crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs`:

**Change the field declaration** (line 88, inside the `#[cfg(feature = "lmdb")]` block):

```rust
// Before:
pub skip_bm25_writes: bool,

// After:
pub skip_bm25_writes: Arc<AtomicBool>,
```

**Add imports** at the top of the lmdb cfg block or file:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
```

**Change `new()` signature** — find `pub fn new(path: &str, config: Config, version_info: VersionInfo)` and add the parameter:

```rust
pub fn new(
    path: &str,
    config: Config,
    version_info: VersionInfo,
    skip_bm25_on_write: Option<Arc<AtomicBool>>,
) -> Result<SparrowGraphStorage, GraphError> {
```

**Change the initialization** (lines 248-251):

```rust
// Before:
let skip_bm25_writes = matches!(
    std::env::var("SPARROW_SKIP_BM25_ON_WRITE").as_deref(),
    Ok("true") | Ok("1")
);

// After:
let skip_bm25_writes = skip_bm25_on_write.unwrap_or_else(|| {
    let from_env = matches!(
        std::env::var("SPARROW_SKIP_BM25_ON_WRITE").as_deref(),
        Ok("true") | Ok("1")
    );
    Arc::new(AtomicBool::new(from_env))
});
```

- [ ] **Step 5: Update the 6 use sites to use `.load(Ordering::Relaxed)`**

Each site currently reads: `!storage.skip_bm25_writes` or `!self.storage.skip_bm25_writes`.

Change all 6 occurrences:

**`ops/util/drop.rs` (1 site):**
```rust
// Before:
if let Some(bm25) = storage.bm25.as_ref().filter(|_| !storage.skip_bm25_writes)
// After:
if let Some(bm25) = storage.bm25.as_ref().filter(|_| !storage.skip_bm25_writes.load(Ordering::Relaxed))
```
Add `use std::sync::atomic::Ordering;` at the top if not already present.

**`ops/util/update.rs` (1 site):**
```rust
// Before:
if let Some(bm25) = self.storage.bm25.as_ref().filter(|_| !self.storage.skip_bm25_writes) {
// After:
if let Some(bm25) = self.storage.bm25.as_ref().filter(|_| !self.storage.skip_bm25_writes.load(Ordering::Relaxed)) {
```

**`ops/util/upsert.rs` (2 sites, lines 287 and 354):**
```rust
// Before (both):
if let Some(bm25) = self.storage.bm25.as_ref().filter(|_| !self.storage.skip_bm25_writes)
// After (both):
if let Some(bm25) = self.storage.bm25.as_ref().filter(|_| !self.storage.skip_bm25_writes.load(Ordering::Relaxed))
```

**`ops/source/add_n.rs` (2 sites, lines 122 and 218):**
```rust
// Before (both):
if let Some(bm25) = self.storage.bm25.as_ref().filter(|_| !self.storage.skip_bm25_writes)
// After (both):
if let Some(bm25) = self.storage.bm25.as_ref().filter(|_| !self.storage.skip_bm25_writes.load(Ordering::Relaxed))
```

- [ ] **Step 6: Run the compile-time test**

```bash
cargo test --package sparrow-core --features lmdb -- skip_bm25_tests
```

Expected: PASS — the field is now `Arc<AtomicBool>`.

- [ ] **Step 7: Run the full test suite to ensure nothing regressed**

```bash
cargo test --workspace --features lmdb,server
```

Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/sparrow-core/src/sparrow_engine/traversal_core/mod.rs \
        crates/sparrow-core/src/sparrow_engine/storage_core/mod.rs \
        crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/drop.rs \
        crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/update.rs \
        crates/sparrow-core/src/sparrow_engine/traversal_core/ops/util/upsert.rs \
        crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_n.rs
git commit -m "feat(storage): change skip_bm25_writes to Arc<AtomicBool> for runtime hot-swap"
```

---

### Task 3: Thread `RuntimeSettings` through Gateway and AppState

**Files:**
- Modify: `crates/sparrow-core/src/sparrow_gateway/gateway.rs`

Add `settings: Arc<RuntimeSettings>` to `AppState` and thread it through `SparrowGateway::new()`.

- [ ] **Step 1: Write the failing test**

In `gateway.rs` tests (or add an inline test), verify `AppState` has a `settings` field:

```rust
#[cfg(test)]
mod settings_field_test {
    use super::*;
    use crate::sparrow_gateway::settings::RuntimeSettings;
    use std::sync::Arc;

    fn assert_app_state_has_settings(_: &Arc<RuntimeSettings>, _: &AppState) {}
    // If AppState doesn't have a `settings` field, this won't compile.
    fn check(state: &AppState) {
        assert_app_state_has_settings(&state.settings, state);
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo build --package sparrow-core --features lmdb 2>&1 | grep "no field"
```

Expected: compile error — `settings` field not found.

- [ ] **Step 3: Add `settings` to `AppState`**

In `gateway.rs`, find `pub struct AppState` and add the field:

```rust
use crate::sparrow_gateway::settings::RuntimeSettings;

pub struct AppState {
    pub worker_pool: WorkerPool,
    pub schema_json: Option<Bytes>,
    pub cluster_id: Option<String>,
    pub settings: Arc<RuntimeSettings>,  // ← add
    #[cfg(feature = "lmdb")]
    pub token_store: Arc<TokenStore>,
}
```

- [ ] **Step 4: Add `settings` parameter to `SparrowGateway::new()`**

```rust
pub fn new(
    address: &str,
    graph_access: Arc<SparrowGraphEngine>,
    workers_per_core: usize,
    routes: Option<HashMap<String, HandlerFn>>,
    mcp_routes: Option<HashMap<String, MCPHandlerFn>>,
    write_routes: Option<HashSet<String>>,
    opts: Option<SparrowGraphEngineOpts>,
    settings: Arc<RuntimeSettings>,  // ← add
) -> SparrowGateway {
```

Store it in the `SparrowGateway` struct — add the field:

```rust
pub struct SparrowGateway {
    pub(crate) address: String,
    pub(crate) workers_per_core: usize,
    pub(crate) graph_access: Arc<SparrowGraphEngine>,
    pub(crate) router: Arc<SparrowRouter>,
    pub(crate) opts: Option<SparrowGraphEngineOpts>,
    pub(crate) cluster_id: Option<String>,
    pub(crate) settings: Arc<RuntimeSettings>,  // ← add
    #[cfg(feature = "lmdb")]
    pub(crate) token_store: Arc<TokenStore>,
}
```

Update the `SparrowGateway { ... }` constructor body to include `settings`.

- [ ] **Step 5: Thread `settings` into `AppState` in `run()`**

In `SparrowGateway::run()`, find where `AppState` is constructed (around line 229):

```rust
// Before:
let axum_app = axum_app.with_state(Arc::new(AppState {
    worker_pool,
    schema_json: self.opts.and_then(|o| o.config.schema.map(Bytes::from)),
    cluster_id: self.cluster_id,
    #[cfg(feature = "lmdb")]
    token_store: Arc::clone(&self.token_store),
}));

// After:
let axum_app = axum_app.with_state(Arc::new(AppState {
    worker_pool,
    schema_json: self.opts.and_then(|o| o.config.schema.map(Bytes::from)),
    cluster_id: self.cluster_id,
    settings: Arc::clone(&self.settings),  // ← add
    #[cfg(feature = "lmdb")]
    token_store: Arc::clone(&self.token_store),
}));
```

- [ ] **Step 6: Build to find all remaining compilation errors**

```bash
cargo build --package sparrow-core --features lmdb 2>&1 | head -40
```

This will show the call site in `main.rs` that needs updating (Task 5 will fix it, but note the error here).

- [ ] **Step 7: Run tests**

```bash
cargo test --package sparrow-core --features lmdb,server -- settings_field_test
```

Expected: PASS after main.rs is updated in Task 5. For now, build should succeed for the sparrow-core package in isolation.

- [ ] **Step 8: Commit**

```bash
git add crates/sparrow-core/src/sparrow_gateway/gateway.rs
git commit -m "feat(gateway): add RuntimeSettings to AppState and SparrowGateway constructor"
```

---

### Task 4: `GET /settings` and `POST /settings` Handlers

**Files:**
- Create: `crates/sparrow-core/src/sparrow_gateway/builtin/settings_handler.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/gateway.rs`

`GET /settings` returns all settings with value, source, and mutability. Any authenticated role can call it.  
`POST /settings` applies a partial update and requires Admin. Unknown or immutable keys return 400.

- [ ] **Step 1: Write the failing tests**

Create `settings_handler.rs` with tests first:

```rust
#[cfg(test)]
mod handler_tests {
    use super::*;

    #[test]
    fn apply_patch_accepts_known_mutable_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"skip_bm25_on_write": true});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_ok());
        assert!(settings.skip_bm25_on_write.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn apply_patch_rejects_immutable_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"worker_threads": 16});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("immutable"));
    }

    #[test]
    fn apply_patch_rejects_unknown_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"unknown_setting": true});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown setting"));
    }
}
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cargo test --package sparrow-core --features lmdb -- handler_tests 2>&1 | head -10
```

Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Make `extract_verified_admin` pub(crate) in token_mgmt.rs**

In `crates/sparrow-core/src/sparrow_gateway/builtin/token_mgmt.rs`, find line 70:

```rust
// Before:
#[cfg(feature = "lmdb")]
fn extract_verified_admin(

// After:
#[cfg(feature = "lmdb")]
pub(crate) fn extract_verified_admin(
```

- [ ] **Step 4: Implement `settings_handler.rs`**

```rust
// crates/sparrow-core/src/sparrow_gateway/builtin/settings_handler.rs

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::{
    protocol::SparrowError,
    sparrow_gateway::{
        auth::Role,
        gateway::AppState,
        settings::RuntimeSettings,
    },
};

/// Apply a partial settings patch from a JSON object.
///
/// Returns `Ok(())` on success, `Err(message)` on validation failure.
/// This is a pure function — no HTTP concerns — making it testable without axum.
pub fn apply_settings_patch(
    settings: &Arc<RuntimeSettings>,
    patch: &serde_json::Value,
) -> Result<(), String> {
    let obj = patch
        .as_object()
        .ok_or_else(|| "request body must be a JSON object".to_string())?;

    for (key, value) in obj {
        match key.as_str() {
            "skip_bm25_on_write" => {
                let v = value
                    .as_bool()
                    .ok_or_else(|| "skip_bm25_on_write must be a boolean".to_string())?;
                settings.set_skip_bm25_on_write(v);
            }
            "worker_threads" => {
                return Err(
                    "setting 'worker_threads' is immutable — restart the container to change it"
                        .to_string(),
                );
            }
            other => {
                return Err(format!("unknown setting '{other}'"));
            }
        }
    }
    Ok(())
}

/// `GET /settings` — return all settings with value, source, and mutability.
/// Requires any authenticated role (or no auth if auth is disabled).
#[cfg(feature = "lmdb")]
pub async fn get_settings_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    // Auth check: any valid token (or no auth required)
    if state.token_store.is_auth_required() {
        let raw_key = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if state.token_store.verify(raw_key).is_err() {
            return SparrowError::InvalidApiKey.into_response();
        }
    }

    let json = state.settings.to_json();
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json,
    )
        .into_response()
}

/// `POST /settings` — apply partial update to mutable settings.
/// Requires Admin role.
#[cfg(feature = "lmdb")]
pub async fn post_settings_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    // Auth check: Admin only
    use crate::sparrow_gateway::builtin::token_mgmt::extract_verified_admin;
    if let Err(e) = extract_verified_admin(&state, &headers) {
        return e;
    }

    let patch: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [("content-type", "application/json")],
                format!(r#"{{"error":"invalid JSON: {e}"}}"#),
            )
                .into_response();
        }
    };

    match apply_settings_patch(&state.settings, &patch) {
        Ok(()) => {
            let json = state.settings.to_json();
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                json,
            )
                .into_response()
        }
        Err(msg) => (
            StatusCode::BAD_REQUEST,
            [("content-type", "application/json")],
            format!(r#"{{"error":"{msg}"}}"#),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "lmdb"))]
pub async fn get_settings_handler() -> axum::response::Response {
    (StatusCode::NOT_IMPLEMENTED, "lmdb feature required").into_response()
}

#[cfg(not(feature = "lmdb"))]
pub async fn post_settings_handler() -> axum::response::Response {
    (StatusCode::NOT_IMPLEMENTED, "lmdb feature required").into_response()
}

#[cfg(test)]
mod handler_tests {
    use super::*;

    #[test]
    fn apply_patch_accepts_known_mutable_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"skip_bm25_on_write": true});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_ok());
        assert!(settings.skip_bm25_on_write.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn apply_patch_rejects_immutable_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"worker_threads": 16});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("immutable"));
    }

    #[test]
    fn apply_patch_rejects_unknown_key() {
        use crate::sparrow_gateway::settings::RuntimeSettings;
        use std::sync::Arc;

        let settings = Arc::new(RuntimeSettings::from_env());
        let patch = serde_json::json!({"unknown_setting": true});
        let result = apply_settings_patch(&settings, &patch);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown setting"));
    }
}
```

- [ ] **Step 5: Register the module in `builtin/mod.rs`**

In `crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs`, add:

```rust
pub mod settings_handler;
```

- [ ] **Step 6: Register the routes in `gateway.rs`**

In `SparrowGateway::run()`, in the `#[cfg(feature = "lmdb")]` block where `/tokens` routes are registered, add:

```rust
#[cfg(feature = "lmdb")]
{
    use crate::sparrow_gateway::builtin::settings_handler::{
        get_settings_handler, post_settings_handler,
    };
    use crate::sparrow_gateway::builtin::token_mgmt::{
        create_token_handler, list_tokens_handler, revoke_token_handler,
    };
    use axum::routing::delete;
    axum_app = axum_app
        .route("/tokens", get(list_tokens_handler).post(create_token_handler))
        .route("/tokens/{id}", delete(revoke_token_handler))
        .route("/settings", get(get_settings_handler).post(post_settings_handler));  // ← add
}
```

- [ ] **Step 7: Run the handler tests**

```bash
cargo test --package sparrow-core --features lmdb -- handler_tests
```

Expected: All 3 tests PASS.

- [ ] **Step 8: Run full test suite**

```bash
cargo test --workspace --features lmdb,server
```

Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/sparrow-core/src/sparrow_gateway/builtin/settings_handler.rs \
        crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs \
        crates/sparrow-core/src/sparrow_gateway/builtin/token_mgmt.rs \
        crates/sparrow-core/src/sparrow_gateway/gateway.rs
git commit -m "feat(gateway): add GET /settings and POST /settings handlers with auth gating"
```

---

### Task 5: Wire Everything Together in `main.rs`

**Files:**
- Modify: `crates/sparrow-container/src/main.rs`

Connect RuntimeSettings creation in `main.rs`, pass the `Arc<AtomicBool>` to `SparrowGraphEngineOpts`, pass `Arc<RuntimeSettings>` to `SparrowGateway::new()`, and remove the now-redundant inline `SPARROW_SKIP_BM25_ON_WRITE` log message.

- [ ] **Step 1: Write the integration test**

The test is a build-level check: if `main.rs` passes the wrong type to `SparrowGateway::new()` or is missing a parameter, the build fails. Verify by building:

```bash
cargo build --package sparrow-container --features lmdb 2>&1 | head -20
```

Expected before change: compilation errors about wrong argument count in `SparrowGateway::new()`.

- [ ] **Step 2: Update `main.rs`**

Add the import at the top of `main.rs`:

```rust
use sparrow_db::sparrow_gateway::settings::RuntimeSettings;
```

After the `config` is parsed and before the `opts` are constructed (around line 75), replace the existing `SPARROW_SKIP_BM25_ON_WRITE` log block and add RuntimeSettings construction:

```rust
// Before (lines 75-80):
if matches!(std::env::var("SPARROW_SKIP_BM25_ON_WRITE").as_deref(), Ok("true") | Ok("1")) {
    println!(
        "\tSPARROW_SKIP_BM25_ON_WRITE=true — BM25 index updates DISABLED during writes. \
         Run POST /rebuild_bm25_index after bulk import to rebuild the index."
    );
}

// After — replace with:
let settings = std::sync::Arc::new(RuntimeSettings::from_env());
if settings.skip_bm25_on_write.load(std::sync::atomic::Ordering::Relaxed) {
    println!(
        "\tSPARROW_SKIP_BM25_ON_WRITE=true — BM25 index updates DISABLED during writes. \
         Run POST /rebuild_bm25_index after bulk import to rebuild the index."
    );
}
```

Update the `SparrowGraphEngineOpts` construction (around line 122):

```rust
// Before:
let opts = SparrowGraphEngineOpts {
    path: path_str.to_string(),
    config,
    version_info: VersionInfo(transition_fns),
};

// After:
let opts = SparrowGraphEngineOpts {
    path: path_str.to_string(),
    config,
    version_info: VersionInfo(transition_fns),
    skip_bm25_on_write: Some(std::sync::Arc::clone(&settings.skip_bm25_on_write)),
};
```

Update the `SparrowGateway::new()` call (around line 185):

```rust
// Before:
let gateway = SparrowGateway::new(
    &format!("0.0.0.0:{port}"),
    graph,
    GatewayOpts::DEFAULT_WORKERS_PER_CORE,
    Some(query_routes),
    Some(mcp_routes),
    Some(write_routes),
    Some(opts),
);

// After:
let gateway = SparrowGateway::new(
    &format!("0.0.0.0:{port}"),
    graph,
    GatewayOpts::DEFAULT_WORKERS_PER_CORE,
    Some(query_routes),
    Some(mcp_routes),
    Some(write_routes),
    Some(opts),
    std::sync::Arc::clone(&settings),  // ← add
);
```

- [ ] **Step 3: Build to verify no errors**

```bash
cargo build --package sparrow-container --features lmdb
```

Expected: Builds successfully.

- [ ] **Step 4: Run the full workspace test suite**

```bash
cargo test --workspace --features lmdb,server
```

Expected: All tests pass.

- [ ] **Step 5: Smoke-test manually**

Start the container locally and verify:

```bash
# Start a sparrow instance (adjust port/path for your environment)
# Then:
curl http://localhost:6969/settings
# Expected: {"settings":{"skip_bm25_on_write":{"value":false,"source":"default","mutable":true},...}}

curl -X POST http://localhost:6969/settings \
  -H "content-type: application/json" \
  -d '{"skip_bm25_on_write": true}'
# Expected: 200 with updated settings showing "source":"runtime"

curl http://localhost:6969/settings
# Expected: skip_bm25_on_write now shows value:true, source:"runtime"

curl -X POST http://localhost:6969/settings \
  -H "content-type: application/json" \
  -d '{"worker_threads": 16}'
# Expected: 400 with error about immutable setting

curl -X POST http://localhost:6969/settings \
  -H "content-type: application/json" \
  -d '{"unknown_key": true}'
# Expected: 400 with error about unknown setting
```

- [ ] **Step 6: Commit**

```bash
git add crates/sparrow-container/src/main.rs
git commit -m "feat(container): wire RuntimeSettings into main — skip_bm25_on_write hot-swappable via POST /settings"
```

---

## Self-Review Checklist

After all tasks complete:

1. `cargo build --workspace --features lmdb,server` — no errors
2. `cargo test --workspace --features lmdb,server` — all pass
3. `GET /settings` returns valid JSON with both settings
4. `POST /settings {"skip_bm25_on_write": true}` toggles the value and shows `"source":"runtime"`
5. After toggling via POST, the next write actually skips BM25 (verify via `GET /diagnostics` and timing)
6. `POST /settings {"worker_threads": 4}` returns 400
7. `POST /settings {"bad_key": true}` returns 400
8. After container restart, `GET /settings` shows `"source":"default"` or `"env"` (not `"runtime"`) — ephemeral confirmed
