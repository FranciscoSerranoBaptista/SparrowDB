# Runtime Settings API Design

**Date:** 2026-05-25  
**Status:** Approved  
**Scope:** `RuntimeSettings` store, `GET /settings`, `POST /settings`, auth integration, boot sequence

---

## Problem

SparrowDB's operational settings are currently locked to environment variables that are read once at startup. Changing any setting — even a safe, non-structural one like `SPARROW_SKIP_BM25_ON_WRITE` — requires restarting the container. In production graph deployments this means a full service interruption to toggle a write-path optimisation.

The simorgh deployment hardcodes options in `.env` files. There is no way to inspect current settings, no way to change a setting without a restart, and no machine-readable format for CI or scripting.

**Invariant to enforce:** Env vars remain the source of truth at startup. Runtime changes are ephemeral — they survive until the next restart, not across restarts. No persistence layer, no config drift.

---

## Design

### 1. `RuntimeSettings` store

A new struct in `crates/sparrow-core/src/sparrow_gateway/settings.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Hot-swappable operational settings for a running SparrowDB instance.
///
/// Fields fall into two categories:
///
/// **Mutable** (`Arc<Atomic*>`): can be changed at runtime via `POST /settings`
/// without a restart. Each field is individually addressable; there is no global
/// lock. Readers use `Ordering::Relaxed` — no happens-before requirement between
/// a toggle and a specific write.
///
/// **Immutable** (plain scalars): set from env vars at startup; readable via
/// `GET /settings` for observability but rejected by `POST /settings` with
/// `400 Bad Request`.
///
/// All changes made via `POST /settings` are ephemeral. Env vars remain the
/// source of truth — the next container restart restores them.
#[derive(Clone, Debug)]
pub struct RuntimeSettings {
    // ── mutable ──────────────────────────────────────────────────────────
    /// Skip BM25 index rebuild on every write. Equivalent to
    /// `SPARROW_SKIP_BM25_ON_WRITE=1`. Toggle during bulk import to trade
    /// search freshness for write throughput, then call `POST /rebuild_bm25_index`.
    pub skip_bm25_on_write: Arc<AtomicBool>,

    // ── immutable (observability only) ───────────────────────────────────
    /// Number of worker threads in the gateway pool. Set by
    /// `SPARROW_WORKER_THREADS` at startup. Cannot be changed at runtime
    /// (OS thread pool is not hot-swappable).
    pub worker_threads: usize,
}

impl RuntimeSettings {
    /// Construct from environment variables, applying defaults where not set.
    pub fn from_env() -> Self {
        let skip = std::env::var("SPARROW_SKIP_BM25_ON_WRITE")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        let workers = std::env::var("SPARROW_WORKER_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(num_cpus::get);

        RuntimeSettings {
            skip_bm25_on_write: Arc::new(AtomicBool::new(skip)),
            worker_threads: workers,
        }
    }
}
```

**Threading through the stack:**

`RuntimeSettings` is constructed once in `main.rs` and passed to both `SparrowGraphEngineOpts` and `GatewayOpts`. Both already hold `Arc`-wrapped shared state; `RuntimeSettings` follows the same pattern. The `Arc<AtomicBool>` fields are cheaply cloned — sharing the underlying allocation, not copying the value.

`storage_core` reads `skip_bm25_on_write` as:
```rust
// Before:
let skip = std::env::var("SPARROW_SKIP_BM25_ON_WRITE").is_ok();

// After:
let skip = self.settings.skip_bm25_on_write.load(Ordering::Relaxed);
```

---

### 2. Endpoint design

**`GET /settings`**

Returns all settings with value, source, and mutability annotation. No auth gate beyond the existing read-tier auth.

```
GET /settings
Authorization: Bearer <read-key-or-higher>

200 OK
{
  "settings": {
    "skip_bm25_on_write": {
      "value": false,
      "source": "default",
      "mutable": true
    },
    "worker_threads": {
      "value": 8,
      "source": "env",
      "mutable": false
    }
  }
}
```

`source` values:
- `"env"` — value originated from an environment variable at startup
- `"default"` — no env var was set; value is the compiled-in default
- `"runtime"` — value was changed via `POST /settings` this session

**`POST /settings`**

Partial merge: only keys present in the request body are updated. Unknown or immutable keys return `400 Bad Request`. Returns the full settings state after the change (same shape as `GET /settings`).

```
POST /settings
Authorization: Bearer <admin-key>
Content-Type: application/json

{ "skip_bm25_on_write": true }

200 OK
{
  "settings": {
    "skip_bm25_on_write": {
      "value": true,
      "source": "runtime",
      "mutable": true
    },
    "worker_threads": {
      "value": 8,
      "source": "env",
      "mutable": false
    }
  }
}
```

Error cases:

```
{ "worker_threads": 16 }
→ 400 Bad Request
  { "error": "setting 'worker_threads' is immutable — restart the container to change it" }

{ "unknown_key": true }
→ 400 Bad Request
  { "error": "unknown setting 'unknown_key'" }
```

Changes applied via `POST /settings` set `source` to `"runtime"` in the serialisation layer. The source is not stored in the `AtomicBool` itself — it is a separate `Arc<Mutex<SettingSource>>` per mutable field, updated alongside the atomic.

---

### 3. Auth and boot integration

**Auth levels**

`GET /settings` requires the read tier (`Authorization: Bearer <read-key>` or higher). It sits with `/diagnostics` and `/health` as an observability endpoint.

`POST /settings` requires the admin tier. The gateway already uses token-based auth with three tiers (read, write, admin). `POST /settings` is added to the admin route group alongside admin-only operations like `POST /rebuild_bm25_index`.

**Boot sequence (3-line change in `main.rs`)**

```rust
// Before (scattered env::var calls at call sites):
let engine_opts = SparrowGraphEngineOpts { ... };
let gateway_opts = GatewayOpts { ... };

// After:
let settings = Arc::new(RuntimeSettings::from_env());
let engine_opts = SparrowGraphEngineOpts { settings: settings.clone(), ... };
let gateway_opts = GatewayOpts { settings: settings.clone(), ... };
```

`RuntimeSettings` derives `Clone`; cloning it clones the `Arc`s (not the values), so all consumers share the same live atomic state.

**No persistence**

`POST /settings` changes survive until the next restart. No file is written, no LMDB key is set. The env var — or its absence — remains the authoritative default. This is a deliberate constraint: it prevents config drift and avoids the complexity of a reconciliation layer at startup.

If a setting should persist, the operator sets the env var and restarts. `GET /settings` `source: "runtime"` is an explicit signal that the running state differs from the persisted env var.

---

## Non-goals

- No persistence of `POST /settings` changes across restarts.
- No `PATCH /settings` — `POST` with partial JSON is sufficient and avoids an extra HTTP verb.
- No settings for LMDB parameters (`map_size`, read workers beyond `worker_threads`) — structural storage settings are not safe to change at runtime.
- No settings WebSocket or long-poll — polling `GET /settings` is sufficient for dashboards.

---

## Files created or modified

| File | Change |
|------|--------|
| `crates/sparrow-core/src/sparrow_gateway/settings.rs` | New — `RuntimeSettings` struct, `from_env()`, source tracking |
| `crates/sparrow-core/src/sparrow_gateway/mod.rs` | Modified — register `settings` module |
| `crates/sparrow-core/src/sparrow_gateway/routes/settings.rs` | New — `GET /settings` and `POST /settings` handlers |
| `crates/sparrow-core/src/sparrow_gateway/routes/mod.rs` | Modified — register settings routes |
| `crates/sparrow-core/src/sparrow_gateway/gateway_opts.rs` | Modified — add `settings: Arc<RuntimeSettings>` field |
| `crates/sparrow-core/src/storage_core/mod.rs` | Modified — read `skip_bm25_on_write` from `RuntimeSettings` instead of `env::var` |
| `crates/sparrow-container/src/main.rs` | Modified — construct `RuntimeSettings::from_env()`, thread through opts |
