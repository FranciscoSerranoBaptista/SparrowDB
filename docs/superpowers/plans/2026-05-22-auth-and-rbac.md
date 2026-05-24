# Authentication & Role-Based Access Control Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single-global-API-key system with a multi-tenant named token store that enforces Admin / ReadWrite / ReadOnly roles on every gateway endpoint.

**Architecture:** A new `auth` module in `sparrow_gateway` owns a `TokenStore` backed by a dedicated LMDB environment at `{data_parent}/auth/`. Auth is always compiled but self-disables when no tokens exist (dev mode). The gateway's `AppState` carries `Arc<TokenStore>`; every Axum handler calls `token_store.verify()` before dispatching to the worker pool. Token management is exposed as three built-in REST routes (`GET/POST /tokens`, `DELETE /tokens/:id`) gated behind the Admin role.

**Tech Stack:** Rust, heed3 (LMDB — already a dep), sha2 (already a dep), rand 0.9 (already a dep), serde (already a dep), axum (already a dep).

---

## File Structure

**New files:**
- `crates/sparrow-core/src/sparrow_gateway/auth/mod.rs` — `Role`, `TokenRecord`, `TokenError` types
- `crates/sparrow-core/src/sparrow_gateway/auth/token_store.rs` — `TokenStore`: LMDB-backed CRUD + `verify()`
- `crates/sparrow-core/src/sparrow_gateway/builtin/token_mgmt.rs` — Axum handlers for token management REST API

**Modified files:**
- `crates/sparrow-core/src/sparrow_gateway/mod.rs` — add `pub mod auth`
- `crates/sparrow-core/src/sparrow_gateway/gateway.rs` — add `token_store` to `SparrowGateway` + `AppState`; replace `verify_key`; register token mgmt routes
- `crates/sparrow-core/src/protocol/error.rs` — add `Forbidden` variant; fix `InvalidApiKey` → 401
- `crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs` — add `pub mod token_mgmt`
- `crates/sparrow-core/src/sparrow_gateway/introspect_schema.rs` — replace `verify_key` with `token_store.verify()`
- `crates/sparrow-core/src/sparrow_gateway/v1_compat/mod.rs` — add auth check to `v1_query_axum_handler`
- `crates/sparrow-core/src/protocol/request.rs` — remove `#[cfg(feature = "api-key")]` guards; always extract `x-api-key`

**Deleted files:**
- `crates/sparrow-core/src/sparrow_gateway/key_verification.rs`

---

## Background you must read before starting

Read these files first — they are short and the plan references them directly:

- `crates/sparrow-core/src/sparrow_gateway/gateway.rs` — `SparrowGateway`, `AppState`, `post_handler`
- `crates/sparrow-core/src/sparrow_gateway/key_verification.rs` — the system being replaced
- `crates/sparrow-core/src/protocol/request.rs` — `Request` struct, `#[cfg(feature = "api-key")]` blocks
- `crates/sparrow-core/src/protocol/error.rs` — `SparrowError` enum
- `crates/sparrow-core/src/sparrow_gateway/introspect_schema.rs` — second auth call site
- `crates/sparrow-core/src/sparrow_gateway/v1_compat/mod.rs` lines 55–75 — third auth call site
- `crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs` — where to add the new module
- `crates/sparrow-core/Cargo.toml` `[features]` section

---

### Task 1: Auth types and TokenStore

**Files:**
- Create: `crates/sparrow-core/src/sparrow_gateway/auth/mod.rs`
- Create: `crates/sparrow-core/src/sparrow_gateway/auth/token_store.rs`

- [ ] **Step 1: Write the failing unit tests for TokenStore**

Create `crates/sparrow-core/src/sparrow_gateway/auth/token_store.rs` with the tests first:

```rust
use super::*;
use tempfile::TempDir;

fn temp_store() -> (TokenStore, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = TokenStore::open(dir.path().to_str().unwrap()).unwrap();
    (store, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_disabled_when_empty() {
        let (store, _dir) = temp_store();
        assert!(!store.is_auth_required());
    }

    #[test]
    fn test_create_and_verify_token() {
        let (store, _dir) = temp_store();
        let (raw, record) = store.create("ci-bot", Role::ReadWrite).unwrap();
        assert_eq!(record.role, Role::ReadWrite);
        assert_eq!(record.name, "ci-bot");

        let verified = store.verify(&raw).unwrap();
        assert_eq!(verified.id, record.id);
    }

    #[test]
    fn test_invalid_token_rejected() {
        let (store, _dir) = temp_store();
        store.create("test", Role::ReadOnly).unwrap();
        let err = store.verify("sparrow_bad_key").unwrap_err();
        assert!(matches!(err, TokenError::InvalidKey));
    }

    #[test]
    fn test_list_tokens() {
        let (store, _dir) = temp_store();
        store.create("alpha", Role::Admin).unwrap();
        store.create("beta", Role::ReadOnly).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_revoke_token() {
        let (store, _dir) = temp_store();
        let (raw, record) = store.create("disposable", Role::ReadWrite).unwrap();
        assert!(store.revoke(&record.id).unwrap());
        assert!(store.verify(&raw).is_err());
        // revoking again returns false
        assert!(!store.revoke(&record.id).unwrap());
    }

    #[test]
    fn test_legacy_sparrow_api_key_seeded_as_admin() {
        let (store, _dir) = temp_store();
        let key = "my-test-legacy-key";
        store.seed_legacy(key);
        let record = store.verify(key).unwrap();
        assert_eq!(record.role, Role::Admin);
        assert_eq!(record.name, "SPARROW_API_KEY");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p sparrow-core --features lmdb auth::token_store 2>&1 | head -40
```

Expected: compile error — module `auth` does not exist yet.

- [ ] **Step 3: Create `auth/mod.rs` with the shared types**

Create `crates/sparrow-core/src/sparrow_gateway/auth/mod.rs`:

```rust
pub mod token_store;
pub use token_store::TokenStore;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Admin,
    ReadWrite,
    ReadOnly,
}

impl Role {
    /// Returns true if this role has at least write access.
    pub fn can_write(&self) -> bool {
        matches!(self, Role::Admin | Role::ReadWrite)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    /// 8-char hex prefix of the SHA-256 hash — stable short ID for listing/revoking.
    pub id: String,
    /// Human-readable label set at creation time.
    pub name: String,
    pub role: Role,
    /// Unix timestamp (seconds) when the token was created.
    pub created_at: u64,
}

#[derive(Debug)]
pub enum TokenError {
    /// No key provided when auth is required.
    Unauthorized,
    /// Key was provided but does not match any stored token.
    InvalidKey,
    /// Key is valid but the role is insufficient for the operation.
    Forbidden,
    Storage(heed3::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Unauthorized => write!(f, "authentication required"),
            TokenError::InvalidKey => write!(f, "invalid API key"),
            TokenError::Forbidden => write!(f, "insufficient permissions"),
            TokenError::Storage(e) => write!(f, "token store error: {e}"),
            TokenError::Io(e) => write!(f, "token store I/O error: {e}"),
            TokenError::Json(e) => write!(f, "token store serialization error: {e}"),
        }
    }
}

impl From<heed3::Error> for TokenError {
    fn from(e: heed3::Error) -> Self { TokenError::Storage(e) }
}
impl From<std::io::Error> for TokenError {
    fn from(e: std::io::Error) -> Self { TokenError::Io(e) }
}
impl From<serde_json::Error> for TokenError {
    fn from(e: serde_json::Error) -> Self { TokenError::Json(e) }
}
```

- [ ] **Step 4: Implement `TokenStore` in `token_store.rs`**

```rust
use std::time::{SystemTime, UNIX_EPOCH};
use heed3::{Database, Env, EnvOpenOptions, byteorder::BE, types::*};
use sha2::{Digest, Sha256};

use super::{Role, TokenError, TokenRecord};

const DB_NAME: &str = "tokens";

pub struct TokenStore {
    env: Env,
    db: Database<Bytes, Bytes>,
}

impl TokenStore {
    pub fn open(path: &str) -> Result<Self, TokenError> {
        std::fs::create_dir_all(path)?;
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(10 * 1024 * 1024) // 10 MB is plenty for tokens
                .max_dbs(1)
                .open(std::path::Path::new(path))?
        };
        let mut wtxn = env.write_txn()?;
        let db = env
            .database_options()
            .types::<Bytes, Bytes>()
            .name(DB_NAME)
            .create(&mut wtxn)?;
        wtxn.commit()?;
        Ok(Self { env, db })
    }

    /// Returns true if auth should be enforced (at least one token exists).
    pub fn is_auth_required(&self) -> bool {
        let rtxn = match self.env.read_txn() {
            Ok(t) => t,
            Err(_) => return false,
        };
        self.db.len(&rtxn).unwrap_or(0) > 0
    }

    /// Verify a raw token string. Returns the associated record on success.
    /// Returns Ok(TokenRecord) if auth is not required (no tokens stored) and key is empty.
    /// — callers that need a real record should check `is_auth_required()` first.
    pub fn verify(&self, raw_key: &str) -> Result<TokenRecord, TokenError> {
        let hash = Self::hash_key(raw_key);
        let rtxn = self.env.read_txn()?;
        match self.db.get(&rtxn, &hash)? {
            Some(bytes) => Ok(serde_json::from_slice(bytes)?),
            None => Err(TokenError::InvalidKey),
        }
    }

    /// Create a new token. Returns `(raw_token_string, record)`.
    /// The raw token is shown once — it is never stored in plaintext.
    pub fn create(&self, name: &str, role: Role) -> Result<(String, TokenRecord), TokenError> {
        let raw = Self::generate_raw();
        let hash = Self::hash_key(&raw);
        let id = hex::encode(&hash[..4]); // 8 hex chars
        let record = TokenRecord {
            id,
            name: name.to_string(),
            role,
            created_at: unix_now(),
        };
        let value = serde_json::to_vec(&record)?;
        let mut wtxn = self.env.write_txn()?;
        self.db.put(&mut wtxn, &hash, &value)?;
        wtxn.commit()?;
        Ok((raw, record))
    }

    /// List all tokens. Returns records (no raw keys — those are never stored).
    pub fn list(&self) -> Result<Vec<TokenRecord>, TokenError> {
        let rtxn = self.env.read_txn()?;
        let mut records = Vec::new();
        for result in self.db.iter(&rtxn)? {
            let (_, value) = result?;
            records.push(serde_json::from_slice(value)?);
        }
        Ok(records)
    }

    /// Revoke a token by its short ID (first 8 hex chars of the SHA-256 hash).
    /// Returns true if a matching token was found and deleted, false if not found.
    pub fn revoke(&self, id: &str) -> Result<bool, TokenError> {
        let rtxn = self.env.read_txn()?;
        // Scan for the record with matching id field (id is the 4-byte hash prefix)
        let target_key_prefix = hex::decode(id).map_err(|_| TokenError::InvalidKey)?;
        if target_key_prefix.len() != 4 {
            return Err(TokenError::InvalidKey);
        }
        let mut found_hash: Option<[u8; 32]> = None;
        for result in self.db.iter(&rtxn)? {
            let (key, value) = result?;
            let record: TokenRecord = serde_json::from_slice(value)?;
            if record.id == id {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(key);
                found_hash = Some(hash);
                break;
            }
        }
        drop(rtxn);
        match found_hash {
            None => Ok(false),
            Some(hash) => {
                let mut wtxn = self.env.write_txn()?;
                self.db.delete(&mut wtxn, &hash)?;
                wtxn.commit()?;
                Ok(true)
            }
        }
    }

    /// Seed the legacy SPARROW_API_KEY as an Admin token (if not already present).
    /// Call this with the raw key value from the env var.
    pub fn seed_legacy(&self, raw_key: &str) {
        let hash = Self::hash_key(raw_key);
        let rtxn = match self.env.read_txn() {
            Ok(t) => t,
            Err(_) => return,
        };
        if self.db.get(&rtxn, &hash).unwrap_or(None).is_some() {
            return; // already seeded
        }
        drop(rtxn);
        let record = TokenRecord {
            id: hex::encode(&hash[..4]),
            name: "SPARROW_API_KEY".to_string(),
            role: Role::Admin,
            created_at: unix_now(),
        };
        if let Ok(value) = serde_json::to_vec(&record) {
            if let Ok(mut wtxn) = self.env.write_txn() {
                let _ = self.db.put(&mut wtxn, &hash, &value);
                let _ = wtxn.commit();
            }
        }
    }

    fn hash_key(raw: &str) -> [u8; 32] {
        Sha256::digest(raw.as_bytes()).into()
    }

    fn generate_raw() -> String {
        let bytes: [u8; 16] = rand::random();
        format!("sparrow_{}", hex::encode(bytes))
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    // (test code from Step 1 goes here)
}
```

Note: `hex` crate is not currently a dependency. Either add it or replace `hex::encode` / `hex::decode` with the inline helpers already used in `key_verification.rs`:

```rust
// Instead of hex::encode(&hash[..4]):
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// Instead of hex::decode(id):
fn hex_to_bytes(s: &str) -> Result<Vec<u8>, TokenError> {
    if s.len() % 2 != 0 { return Err(TokenError::InvalidKey); }
    s.as_bytes()
        .chunks(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap_or("ZZ"), 16)
            .map_err(|_| TokenError::InvalidKey))
        .collect()
}
```

Use these inline helpers instead of the `hex` crate — no new dependency needed.

Also add `serde_json` to `[dependencies]` in `crates/sparrow-core/Cargo.toml` if not already present. Check first:
```bash
grep "serde_json" crates/sparrow-core/Cargo.toml
```
If missing, add: `serde_json = "1"`.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p sparrow-core --features lmdb sparrow_gateway::auth 2>&1 | tail -20
```

Expected: all 6 tests in `auth::token_store::tests` pass.

- [ ] **Step 6: Commit**

```bash
git add crates/sparrow-core/src/sparrow_gateway/auth/
git commit -m "feat(auth): add TokenStore with Role-based named tokens"
```

---

### Task 2: Wire TokenStore into AppState

**Files:**
- Modify: `crates/sparrow-core/src/sparrow_gateway/mod.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/gateway.rs`

- [ ] **Step 1: Write the failing test**

In `crates/sparrow-core/src/sparrow_gateway/tests/gateway_tests.rs`, add at the bottom:

```rust
#[test]
fn test_gateway_has_token_store() {
    let (graph, _temp_dir) = create_test_graph();
    let gateway = SparrowGateway::new("127.0.0.1:8080", graph, 8, None, None, None, None);
    // TokenStore must have been created — verify auth is not required (no tokens seeded)
    assert!(!gateway.token_store.is_auth_required());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p sparrow-core --features lmdb test_gateway_has_token_store 2>&1 | head -20
```

Expected: compile error — `gateway.token_store` does not exist.

- [ ] **Step 3: Add `pub mod auth` to `mod.rs`**

In `crates/sparrow-core/src/sparrow_gateway/mod.rs`, add:

```rust
#[cfg(feature = "lmdb")]
pub mod auth;
```

(Place it after existing `pub mod` declarations.)

- [ ] **Step 4: Add `token_store` to `SparrowGateway` and `AppState` in `gateway.rs`**

At the top of `gateway.rs`, add the import:

```rust
#[cfg(feature = "lmdb")]
use crate::sparrow_gateway::auth::{Role, TokenStore};
```

Change `SparrowGateway` struct from:

```rust
pub struct SparrowGateway {
    pub(crate) address: String,
    pub(crate) workers_per_core: usize,
    pub(crate) graph_access: Arc<SparrowGraphEngine>,
    pub(crate) router: Arc<SparrowRouter>,
    pub(crate) opts: Option<SparrowGraphEngineOpts>,
    pub(crate) cluster_id: Option<String>,
}
```

to:

```rust
pub struct SparrowGateway {
    pub(crate) address: String,
    pub(crate) workers_per_core: usize,
    pub(crate) graph_access: Arc<SparrowGraphEngine>,
    pub(crate) router: Arc<SparrowRouter>,
    pub(crate) opts: Option<SparrowGraphEngineOpts>,
    pub(crate) cluster_id: Option<String>,
    #[cfg(feature = "lmdb")]
    pub(crate) token_store: Arc<TokenStore>,
}
```

Change `AppState` from:

```rust
pub struct AppState {
    pub worker_pool: WorkerPool,
    pub schema_json: Option<Bytes>,
    pub cluster_id: Option<String>,
}
```

to:

```rust
pub struct AppState {
    pub worker_pool: WorkerPool,
    pub schema_json: Option<Bytes>,
    pub cluster_id: Option<String>,
    #[cfg(feature = "lmdb")]
    pub token_store: Arc<TokenStore>,
}
```

- [ ] **Step 5: Create the `TokenStore` inside `SparrowGateway::new()`**

In `SparrowGateway::new()`, after the existing cluster_id line, add:

```rust
#[cfg(feature = "lmdb")]
let token_store = {
    // Derive auth path: sibling of the data directory named "auth"
    let auth_path = opts.as_ref()
        .map(|o| {
            std::path::Path::new(&o.path)
                .parent()
                .unwrap_or_else(|| std::path::Path::new("/tmp"))
                .join("auth")
        })
        .unwrap_or_else(|| {
            // Tests: unique temp path per instance avoids concurrent test conflicts
            let rnd: u64 = rand::random();
            std::path::PathBuf::from(format!("/tmp/sparrow_auth_{rnd:x}"))
        });

    let store = TokenStore::open(
        auth_path.to_str().expect("auth path is valid UTF-8"),
    ).expect("failed to open token store");

    // Seed legacy SPARROW_API_KEY as admin token if set
    if let Ok(legacy_key) = std::env::var("SPARROW_API_KEY") {
        if !legacy_key.is_empty() {
            store.seed_legacy(&legacy_key);
        }
    }

    Arc::new(store)
};
```

Update the struct literal at the end of `SparrowGateway::new()` to include the new field:

```rust
SparrowGateway {
    address: address.to_string(),
    graph_access,
    router,
    workers_per_core,
    opts,
    cluster_id,
    #[cfg(feature = "lmdb")]
    token_store,
}
```

- [ ] **Step 6: Thread `token_store` into `AppState` inside `SparrowGateway::run()`**

In `SparrowGateway::run()`, the `AppState` is constructed like this:

```rust
let axum_app = axum_app.with_state(Arc::new(AppState {
    worker_pool,
    schema_json: self.opts.and_then(|o| o.config.schema.map(Bytes::from)),
    cluster_id: self.cluster_id,
}));
```

Change to:

```rust
let axum_app = axum_app.with_state(Arc::new(AppState {
    worker_pool,
    schema_json: self.opts.and_then(|o| o.config.schema.map(Bytes::from)),
    cluster_id: self.cluster_id,
    #[cfg(feature = "lmdb")]
    token_store: Arc::clone(&self.token_store),
}));
```

- [ ] **Step 7: Run tests to verify they pass**

```bash
cargo test -p sparrow-core --features lmdb test_gateway_has_token_store 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/sparrow-core/src/sparrow_gateway/mod.rs \
        crates/sparrow-core/src/sparrow_gateway/gateway.rs \
        crates/sparrow-core/src/sparrow_gateway/tests/gateway_tests.rs
git commit -m "feat(auth): wire TokenStore into SparrowGateway and AppState"
```

---

### Task 3: Replace auth enforcement in all gateway handlers

**Files:**
- Modify: `crates/sparrow-core/src/protocol/error.rs`
- Modify: `crates/sparrow-core/src/protocol/request.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/gateway.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/introspect_schema.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/v1_compat/mod.rs`
- Delete: `crates/sparrow-core/src/sparrow_gateway/key_verification.rs`

- [ ] **Step 1: Write the failing tests**

In `crates/sparrow-core/src/sparrow_gateway/tests/gateway_tests.rs`, add:

```rust
// These tests require an HTTP server — they live in an integration test file.
// For now, add a unit test verifying the auth helper function directly.

#[test]
fn test_verify_request_no_auth_required() {
    use crate::sparrow_gateway::auth::TokenStore;
    let dir = tempfile::tempdir().unwrap();
    let store = TokenStore::open(dir.path().to_str().unwrap()).unwrap();
    // No tokens → auth disabled → any call passes
    assert!(!store.is_auth_required());
}

#[test]
fn test_verify_request_auth_required_no_key() {
    use crate::sparrow_gateway::auth::{Role, TokenError, TokenStore};
    let dir = tempfile::tempdir().unwrap();
    let store = TokenStore::open(dir.path().to_str().unwrap()).unwrap();
    store.create("test", Role::ReadWrite).unwrap();
    // Auth is now required; empty key should fail
    let err = store.verify("").unwrap_err();
    assert!(matches!(err, TokenError::InvalidKey));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p sparrow-core --features lmdb test_verify_request 2>&1 | head -20
```

Expected: compile error or FAIL — `tempfile` may need to be added as a dev-dependency.

Add to `crates/sparrow-core/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

Re-run — both tests should now pass (these are TokenStore tests, not full gateway tests).

- [ ] **Step 3: Add `Forbidden` to `SparrowError` and fix `InvalidApiKey` HTTP status**

In `crates/sparrow-core/src/protocol/error.rs`, change:

```rust
#[derive(Debug, Error)]
pub enum SparrowError {
    #[error("{0}")]
    Graph(#[from] GraphError),
    #[error("{0}")]
    Vector(#[from] VectorError),
    #[error("Couldn't find `{name}` of type {ty:?}")]
    NotFound { ty: RequestType, name: String },
    #[error("Invalid API key")]
    InvalidApiKey,
}
```

to:

```rust
#[derive(Debug, Error)]
pub enum SparrowError {
    #[error("{0}")]
    Graph(#[from] GraphError),
    #[error("{0}")]
    Vector(#[from] VectorError),
    #[error("Couldn't find `{name}` of type {ty:?}")]
    NotFound { ty: RequestType, name: String },
    #[error("Invalid API key")]
    InvalidApiKey,
    #[error("Insufficient permissions")]
    Forbidden,
}
```

In the `IntoResponse` impl, add the `Forbidden` arm:

```rust
SparrowError::InvalidApiKey => axum::http::StatusCode::UNAUTHORIZED,  // was FORBIDDEN — fix
SparrowError::Forbidden => axum::http::StatusCode::FORBIDDEN,
```

Also add `INVALID_API_KEY` and `FORBIDDEN` codes:

```rust
SparrowError::InvalidApiKey => "INVALID_API_KEY",
SparrowError::Forbidden => "FORBIDDEN",
```

(The `code()` match arm for `Forbidden` just needs to be added alongside the existing arms.)

- [ ] **Step 4: Remove `#[cfg(feature = "api-key")]` from `request.rs`**

In `crates/sparrow-core/src/protocol/request.rs`, the `api_key` extraction currently looks like:

```rust
let api_key = {
    #[cfg(feature = "api-key")]
    match headers.get("x-api-key") {
        Some(v) => match v.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => return Err(StatusCode::BAD_REQUEST),
        },
        None => return Err(StatusCode::BAD_REQUEST),
    }
    #[cfg(not(feature = "api-key"))]
    None::<String>
};
```

Replace the entire `api_key` block with:

```rust
let api_key = match headers.get("x-api-key") {
    Some(v) => match v.to_str() {
        Ok(s) => Some(s.to_string()),
        Err(_) => return Err(StatusCode::BAD_REQUEST),
    },
    None => None,  // missing key is allowed here — auth check happens in the handler
};
```

The key point: we no longer reject missing keys at the HTTP parsing stage. We always extract if present, and the Axum handler decides whether auth is required.

- [ ] **Step 5: Replace `verify_key` in `post_handler` inside `gateway.rs`**

In `post_handler`, remove the entire `#[cfg(feature = "api-key")]` block:

```rust
// REMOVE THIS ENTIRE BLOCK:
#[cfg(feature = "api-key")]
{
    use crate::sparrow_gateway::key_verification::verify_key;
    if let Err(e) = verify_key(req.api_key.as_ref().unwrap()) {
        info!(?e, "Invalid API key");
        sparrow_metrics::log_event(...);
        return e.into_response();
    }
}
```

Replace with:

```rust
#[cfg(feature = "lmdb")]
{
    use crate::sparrow_gateway::auth::TokenError;
    if state.token_store.is_auth_required() {
        let raw_key = req.api_key.as_deref().unwrap_or("");
        match state.token_store.verify(raw_key) {
            Ok(record) => {
                // Write routes require at least ReadWrite role
                if state.router.is_write_route(&req.name) && !record.role.can_write() {
                    return SparrowError::Forbidden.into_response();
                }
            }
            Err(TokenError::InvalidKey) | Err(TokenError::Unauthorized) => {
                sparrow_metrics::log_event(
                    sparrow_metrics::events::EventType::InvalidApiKey,
                    sparrow_metrics::events::InvalidApiKeyEvent {
                        cluster_id: state.cluster_id.clone(),
                        time_taken_usec: start_time.elapsed().as_micros() as u32,
                    },
                );
                return SparrowError::InvalidApiKey.into_response();
            }
            Err(_) => return SparrowError::InvalidApiKey.into_response(),
        }
    }
}
```

- [ ] **Step 6: Replace auth in `introspect_schema_handler`**

In `crates/sparrow-core/src/sparrow_gateway/introspect_schema.rs`, replace:

```rust
pub async fn introspect_schema_handler(
    State(state): State<Arc<AppState>>,
    #[cfg(feature = "api-key")] headers: HeaderMap,
) -> axum::response::Response {
    #[cfg(feature = "api-key")]
    {
        use crate::sparrow_gateway::key_verification::verify_key;
        let api_key = match headers.get("x-api-key") { ... };
        if let Err(e) = verify_key(api_key) { return e.into_response(); }
    }
    ...
}
```

with:

```rust
pub async fn introspect_schema_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    #[cfg(feature = "lmdb")]
    {
        if state.token_store.is_auth_required() {
            let raw_key = headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if let Err(_) = state.token_store.verify(raw_key) {
                use crate::protocol::SparrowError;
                return SparrowError::InvalidApiKey.into_response();
            }
        }
    }

    match state.schema_json.as_ref() {
        Some(data) => axum::response::Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(data.clone()))
            .expect("should be able to make response from string"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Could not find schema").into_response(),
    }
}
```

Remove the `#[cfg(feature = "api-key")]` import of `HeaderMap` — it is now always imported.

- [ ] **Step 7: Add auth to `v1_query_axum_handler`**

In `crates/sparrow-core/src/sparrow_gateway/v1_compat/mod.rs`, change the handler signature from:

```rust
pub async fn v1_query_axum_handler(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> axum::http::Response<Body> {
```

to:

```rust
pub async fn v1_query_axum_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> axum::http::Response<Body> {
```

At the top of the function body, before the `is_write` check, add:

```rust
#[cfg(feature = "lmdb")]
{
    if state.token_store.is_auth_required() {
        let raw_key = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        match state.token_store.verify(raw_key) {
            Ok(record) => {
                if is_write && !record.role.can_write() {
                    use crate::protocol::SparrowError;
                    return SparrowError::Forbidden.into_response();
                }
            }
            Err(_) => {
                use crate::protocol::SparrowError;
                return SparrowError::InvalidApiKey.into_response();
            }
        }
    }
}
```

Note: `is_write` is computed on the line immediately following this block — move the `is_write` computation before the auth block so the role check can use it. The reordered top of `v1_query_axum_handler` becomes:

```rust
let is_write = body.windows(b"\"write\"".len()).any(|w| w == b"\"write\"");
let handler_name = if is_write { "__v1_compat_write" } else { "__v1_compat_read" };

#[cfg(feature = "lmdb")]
{
    if state.token_store.is_auth_required() {
        let raw_key = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        match state.token_store.verify(raw_key) {
            Ok(record) => {
                if is_write && !record.role.can_write() {
                    use crate::protocol::SparrowError;
                    return SparrowError::Forbidden.into_response();
                }
            }
            Err(_) => {
                use crate::protocol::SparrowError;
                return SparrowError::InvalidApiKey.into_response();
            }
        }
    }
}
```

- [ ] **Step 8: Delete `key_verification.rs` and remove its mod declaration**

```bash
rm crates/sparrow-core/src/sparrow_gateway/key_verification.rs
```

In `crates/sparrow-core/src/sparrow_gateway/mod.rs`, remove:

```rust
#[cfg(feature = "api-key")]
pub mod key_verification;
```

- [ ] **Step 9: Remove `api-key` feature from `Cargo.toml`**

In `crates/sparrow-core/Cargo.toml`, in the `[features]` section, remove:

```toml
api-key = []
```

and also remove it from the `production` feature:

```toml
# Before:
production = ["api-key"]

# After:
production = []
```

- [ ] **Step 10: Run full test suite to verify nothing is broken**

```bash
cargo test -p sparrow-core --features lmdb -- --test-threads=2 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 11: Commit**

```bash
git add crates/sparrow-core/src/protocol/error.rs \
        crates/sparrow-core/src/protocol/request.rs \
        crates/sparrow-core/src/sparrow_gateway/gateway.rs \
        crates/sparrow-core/src/sparrow_gateway/introspect_schema.rs \
        crates/sparrow-core/src/sparrow_gateway/v1_compat/mod.rs \
        crates/sparrow-core/src/sparrow_gateway/mod.rs \
        crates/sparrow-core/Cargo.toml
git rm crates/sparrow-core/src/sparrow_gateway/key_verification.rs
git commit -m "feat(auth): replace single API key with TokenStore enforcement on all routes"
```

---

### Task 4: Token management REST handlers

**Files:**
- Create: `crates/sparrow-core/src/sparrow_gateway/builtin/token_mgmt.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs`
- Modify: `crates/sparrow-core/src/sparrow_gateway/gateway.rs`

These handlers are always registered but protected by Admin role. They use Axum's typed JSON extractor — a pattern different from the worker-pool `BasicHandlerFn` functions. Study the `introspect_schema_handler` for the exact axum pattern used in this codebase.

- [ ] **Step 1: Write the failing tests**

In `crates/sparrow-core/src/sparrow_gateway/builtin/token_mgmt.rs`, stub the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Integration tests for the token management API would use an HTTP client.
    // Unit tests here verify the helper that checks admin role.

    #[test]
    fn test_require_admin_passes_for_admin_role() {
        use crate::sparrow_gateway::auth::{Role, TokenRecord};
        let record = TokenRecord {
            id: "aabbccdd".to_string(),
            name: "owner".to_string(),
            role: Role::Admin,
            created_at: 0,
        };
        assert!(require_admin(&record).is_ok());
    }

    #[test]
    fn test_require_admin_rejects_read_write_role() {
        use crate::sparrow_gateway::auth::{Role, TokenRecord};
        use crate::protocol::SparrowError;
        let record = TokenRecord {
            id: "aabbccdd".to_string(),
            name: "writer".to_string(),
            role: Role::ReadWrite,
            created_at: 0,
        };
        assert!(matches!(require_admin(&record), Err(SparrowError::Forbidden)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p sparrow-core --features lmdb token_mgmt 2>&1 | head -20
```

Expected: compile error — module does not exist.

- [ ] **Step 3: Implement the token management handlers**

Create `crates/sparrow-core/src/sparrow_gateway/builtin/token_mgmt.rs`:

```rust
use std::sync::Arc;

use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::{
    protocol::SparrowError,
    sparrow_gateway::{
        auth::{Role, TokenRecord},
        gateway::AppState,
    },
};

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub role: RoleInput,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleInput {
    Admin,
    ReadWrite,
    ReadOnly,
}

impl From<RoleInput> for Role {
    fn from(r: RoleInput) -> Self {
        match r {
            RoleInput::Admin => Role::Admin,
            RoleInput::ReadWrite => Role::ReadWrite,
            RoleInput::ReadOnly => Role::ReadOnly,
        }
    }
}

#[derive(Serialize)]
pub struct CreateTokenResponse {
    pub token: String,   // the raw token — shown once
    pub record: TokenRecord,
}

/// Verify the caller holds an Admin token. Returns Err if not.
pub fn require_admin(record: &TokenRecord) -> Result<(), SparrowError> {
    if record.role == Role::Admin {
        Ok(())
    } else {
        Err(SparrowError::Forbidden)
    }
}

fn extract_verified_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TokenRecord, axum::http::Response<Body>> {
    let raw_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let record = state.token_store.verify(raw_key).map_err(|_| {
        SparrowError::InvalidApiKey.into_response()
    })?;
    require_admin(&record).map_err(|e| e.into_response())?;
    Ok(record)
}

pub async fn list_tokens_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(e) = extract_verified_admin(&state, &headers) {
        return e;
    }
    match state.token_store.list() {
        Ok(records) => Json(records).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn create_token_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateTokenRequest>,
) -> axum::response::Response {
    if let Err(e) = extract_verified_admin(&state, &headers) {
        return e;
    }
    match state.token_store.create(&body.name, body.role.into()) {
        Ok((raw_token, record)) => {
            Json(CreateTokenResponse { token: raw_token, record }).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn revoke_token_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> axum::response::Response {
    if let Err(e) = extract_verified_admin(&state, &headers) {
        return e;
    }
    match state.token_store.revoke(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    // test code from Step 1 goes here
}
```

- [ ] **Step 4: Add `pub mod token_mgmt` to `builtin/mod.rs`**

In `crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs`, add:

```rust
pub mod token_mgmt;
```

This module is NOT gated on `dev-instance` — token management is always available.

- [ ] **Step 5: Register the routes in `gateway.run()`**

In `crates/sparrow-core/src/sparrow_gateway/gateway.rs`, in the `run()` method, add the token management routes to the `axum_app` builder. Add these alongside the other routes:

```rust
use crate::sparrow_gateway::builtin::token_mgmt::{
    create_token_handler, list_tokens_handler, revoke_token_handler,
};

// Inside run(), change axum_app construction:
axum_app = axum_app
    .route("/v1/query", post(v1_query_axum_handler))
    .route("/{*path}", post(post_handler))
    .route("/introspect", get(introspect_schema_handler))
    .route("/tokens", get(list_tokens_handler).post(create_token_handler))
    .route("/tokens/:id", delete(revoke_token_handler));
```

(The `delete` method needs `use axum::routing::delete;` added to the imports at the top of `gateway.rs`.)

- [ ] **Step 6: Run tests to verify they pass**

```bash
cargo test -p sparrow-core --features lmdb token_mgmt 2>&1 | tail -10
```

Expected: `test_require_admin_passes_for_admin_role` and `test_require_admin_rejects_read_write_role` pass.

```bash
cargo test -p sparrow-core --features lmdb -- --test-threads=2 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/sparrow-core/src/sparrow_gateway/builtin/token_mgmt.rs \
        crates/sparrow-core/src/sparrow_gateway/builtin/mod.rs \
        crates/sparrow-core/src/sparrow_gateway/gateway.rs
git commit -m "feat(auth): add token management REST API (POST/GET/DELETE /tokens)"
```

---

### Task 5: Full integration verification

**Files:**
- Read: `crates/sparrow-container/src/main.rs` — no changes needed (derives auth_path from opts which is already passed)

- [ ] **Step 1: Verify sparrow-container compiles**

```bash
cargo build --package sparrow-container --features lmdb 2>&1 | tail -20
```

Expected: clean build. `sparrow-container/main.rs` calls `SparrowGateway::new(..., Some(opts))` — since `auth_path` is now derived from `opts.path` inside `SparrowGateway::new()`, no changes to `main.rs` are needed.

- [ ] **Step 2: Run the full workspace test suite**

```bash
cargo test --workspace --features lmdb -- --test-threads=2 2>&1 | tail -30
```

Expected: all tests pass, no regressions.

- [ ] **Step 3: Smoke-test the token flow manually**

Start a container locally (with Docker) and verify the token management API works end-to-end:

```bash
# Start the instance
sparrow start <instance-name>

# No tokens yet — auth is disabled, all calls succeed without a key
curl -s http://localhost:6969/introspect | jq .

# Create an admin token using the legacy SPARROW_API_KEY (set in .env)
curl -s -X POST http://localhost:6969/tokens \
  -H "x-api-key: $SPARROW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name":"ci-deploy","role":"read_write"}' | jq .
# Expected: {"token":"sparrow_<hex>","record":{"id":"...","name":"ci-deploy","role":"ReadWrite",...}}

# Save the returned token
NEW_TOKEN="sparrow_<hex from above>"

# Verify the new token works
curl -s http://localhost:6969/introspect \
  -H "x-api-key: $NEW_TOKEN" | jq .

# Verify a bad key is rejected with 401
curl -s -o /dev/null -w "%{http_code}" http://localhost:6969/introspect \
  -H "x-api-key: bad-key"
# Expected: 401

# Verify a read-only token is rejected on write routes
READ_TOKEN=$(curl -s -X POST http://localhost:6969/tokens \
  -H "x-api-key: $SPARROW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name":"readonly-agent","role":"read_only"}' | jq -r .token)

curl -s -o /dev/null -w "%{http_code}" http://localhost:6969/<write-route-name> \
  -H "x-api-key: $READ_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}'
# Expected: 403

# List and revoke
curl -s http://localhost:6969/tokens \
  -H "x-api-key: $SPARROW_API_KEY" | jq .
TOKEN_ID="<id from list>"
curl -s -X DELETE http://localhost:6969/tokens/$TOKEN_ID \
  -H "x-api-key: $SPARROW_API_KEY"
# Expected: 204 No Content
```

- [ ] **Step 4: Commit**

```bash
git commit --allow-empty -m "test(auth): verify full auth flow end-to-end"
```

(If no code changes were needed after smoke testing, use `--allow-empty`.)

---

## Self-Review

**Spec coverage check:**
- ✅ Named tokens with roles — Task 1
- ✅ Admin / ReadWrite / ReadOnly enforcement — Tasks 3, 4
- ✅ Write-route gating by role — Task 3 (post_handler, v1_query_axum_handler)
- ✅ Backward compatibility with SPARROW_API_KEY — Task 2 (`seed_legacy`)
- ✅ Token management API — Task 4
- ✅ Dev mode (no tokens = no auth) — Tasks 1, 3 (`is_auth_required()`)
- ✅ All three Axum handler entry points covered — post_handler, introspect, v1_compat

**Type consistency check:**
- `Role::Admin | ReadWrite | ReadOnly` used consistently across `auth/mod.rs`, `token_mgmt.rs`, `RoleInput` conversion
- `TokenRecord.id` is always 8 hex chars (`bytes_to_hex(&hash[..4])`)
- `TokenStore.verify()` takes `&str` (raw key); `seed_legacy` also takes `&str`
- `is_auth_required()` returns `bool` — used as guard before `verify()` in all 3 handlers

**Placeholder scan:**
- No TBDs or TODOs — all code is written out in full
- The smoke test in Task 5 references `<write-route-name>` — replace with an actual write route name from your schema (e.g. the compiled HQL write handler name) during execution
