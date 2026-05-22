# Sparrow Studio — Embedded Web UI Design

**Date:** 2026-05-22  
**Status:** Approved  
**Scope:** v1 — local developer tooling, embedded in `sparrow-container`

---

## 1. Goal

Ship a browser-based Studio UI accessible at `http://localhost:<port>/__studio/` whenever `sparrow-container` is running. Zero install, zero separate process. Covers the five views developers need most: HQL Editor, Schema Browser, Graph Visualiser, Diagnostics, and Vector Index management.

**Not in scope for v1:** authentication management UI (token CRUD), remote deployment, multi-user access, schema migrations.

---

## 2. Architecture

Four components; three already exist.

```
packages/studio/          ← NEW: SolidJS app (Apache-2.0)
crates/sparrow-studio/    ← NEW: Rust embed + Axum route (AGPL-3.0)
crates/sparrow-core/      ← MODIFIED: minor additions only
crates/sparrow-container/ ← MODIFIED: adds optional sparrow-studio dep
```

### 2.1 `packages/studio`

SolidJS + TypeScript + Vite application. Lives in the existing pnpm workspace.  
Package name: `@sparrowdb/studio`  
License: Apache-2.0 (same rule as `sdks/` — no dependency on internal crates, only HTTP calls).

```
packages/studio/
  src/
    App.tsx             ← sidebar shell + routing
    views/
      HqlEditor.tsx
      SchemaBrowser.tsx
      GraphViz.tsx
      Diagnostics.tsx
      Vectors.tsx
    api/
      client.ts         ← typed fetch wrapper, passes x-api-key header
    store/
      connection.ts     ← SolidJS store: baseUrl, apiKey, connected status
    index.tsx
  vite.config.ts        ← dev proxy: /v1/*, /__hql_runtime_eval, /introspect, /… → localhost:2048
  package.json
  tsconfig.json
  dist/                 ← gitignored build output consumed by sparrow-studio crate
```

### 2.2 `crates/sparrow-studio`

New Rust library crate. Owns exactly two things: embedding the `dist/` bundle and serving it.

```
crates/sparrow-studio/
  Cargo.toml
  build.rs              ← cargo:rerun-if-changed=../../packages/studio/dist/
  src/
    lib.rs              ← pub fn studio_router() -> axum::Router
    embed.rs            ← rust-embed Assets struct
    handler.rs          ← GET /__studio/* Axum handler
```

`rust-embed` path: `#[folder = "../../packages/studio/dist/"]` (resolved relative to `CARGO_MANIFEST_DIR`). If `dist/` is absent at compile time, the build fails with a clear error — no silent failures.

Serving rules:
- `GET /__studio` → 301 → `/__studio/`
- `GET /__studio/assets/*` → exact embedded file, `Cache-Control: public, max-age=31536000, immutable`
- `GET /__studio/` and `GET /__studio/*` → `index.html` (SPA fallback)
- No auth check on static files — the UI shell itself is always accessible.

### 2.3 `crates/sparrow-container`

Enables the `studio` feature on `sparrow-core` by default. No router code lives here — `sparrow-container` just calls `SparrowGateway::run()`:

```toml
# crates/sparrow-container/Cargo.toml
[dependencies]
sparrow-core = { path = "../sparrow-core", features = ["lmdb", "studio"] }

# or, to make studio opt-out:
[features]
default = ["studio"]
studio = ["sparrow-core/studio"]
```

Strip studio from a minimal build: `cargo build --package sparrow-container --no-default-features --features lmdb`.

### 2.3a `crates/sparrow-core` — feature and merge point

The `studio` feature and the `axum_app.merge()` call live in `sparrow-core` because that is where the Axum router is assembled (`crates/sparrow-core/src/sparrow_gateway/gateway.rs`):

```toml
# crates/sparrow-core/Cargo.toml
[dependencies]
sparrow-studio = { path = "../sparrow-studio", optional = true }

[features]
studio = ["dep:sparrow-studio"]
```

In `gateway.rs` inside `SparrowGateway::run()`, alongside the other explicit GET routes:

```rust
#[cfg(feature = "studio")]
{
    axum_app = axum_app.merge(sparrow_studio::studio_router());
}
```

### 2.4 `crates/sparrow-core` — minimal changes

Two additions only:

1. **`__hql_runtime_eval` always registered when `studio` feature is active** — currently gated behind `SPARROW_RUNTIME_HQL=true` env var in `crates/sparrow-container/src/main.rs`. When the `studio` feature is compiled in, the env gate is bypassed and the route is unconditionally registered via a `#[cfg(feature = "studio")]` block alongside the existing env-var check. It must be added to `write_routes` in `SparrowRouter` because HQL can execute mutations; this ensures the auth plan's `post_handler` role check gates it correctly behind ReadWrite/Admin.

2. **No other changes** — `/introspect`, `POST /node_details`, `POST /nodes_by_label`, `POST /node_connections`, `POST /diagnostics`, `POST /hnsw_health`, vector endpoints all exist today and are called as-is by the Studio frontend.

---

## 3. Frontend Design

### 3.1 Navigation shell (`App.tsx`)

Expanded sidebar (160px), dark theme (`#0d1117` background, `#161b22` sidebar).

```
┌─────────────────────┐
│ ⬡ Sparrow  Studio   │  ← logo + wordmark
├─────────────────────┤
│ ▶ HQL Editor        │  ← default landing, active item has left border accent
│   Schema            │
│   Graph             │
│   Diagnostics       │
│   Vectors           │
├─────────────────────┤
│ ● localhost:2048    │  ← connection status, opens settings modal on click
└─────────────────────┘
```

Connection state lives in a single SolidJS store (`store/connection.ts`):

```ts
type ConnectionStore = {
  baseUrl: string;       // e.g. "http://localhost:2048"
  apiKey: string;        // empty string when auth is disabled
  connected: boolean;
};
```

Stored in `localStorage` under `sparrow_studio_connection`. On first load, if nothing is stored, the settings modal opens automatically.

### 3.2 View: HQL Editor

- **Top bar:** "Query" label | spacer | Run button | Format button | History dropdown
- **Editor panel (top ~40% of content area):** CodeMirror 6 with a custom HQL grammar (keywords: `V`, `E`, `WHERE`, `TRAVERSE`, `RETURN`, `OUT`, `IN`, `ALIAS`; node/edge type names highlighted in blue, string literals in green). No LSP in v1 — syntax highlighting and bracket matching only.
- **Results panel (bottom ~60%):** Three tabs: Table | JSON | Graph ↗
  - Table: virtualised rows, sortable columns, copy-cell on click
  - JSON: raw pretty-printed response
  - Graph ↗: sends current results to the Graph view's canvas
- **Status bar:** result count + execution time (from response timing)
- **Calls:** `POST /__hql_runtime_eval` with `{ "query": "<hql>", "params": {} }`

### 3.3 View: Schema Browser

- Reads from `GET /introspect` on view mount; cached in store until page reload.
- Two sections: **Node Types** and **Edge Types**, each rendered as a card grid.
- Each card shows: type name (blue for nodes, orange for edges), property list with types.
- Edge cards show `From → To` direction.
- No edit capability in v1 — read-only.

### 3.4 View: Graph Visualiser

Query-driven — no dependency on the dev-instance-only `/nodes-edges` route.

- **Inline query bar** (full width, single-line CodeMirror input) + Run button
- **Canvas:** Cytoscape.js with `cose-bilkent` layout. Nodes coloured by type (derived from schema). Edges labelled by type.
- **Selected node panel** (top-right overlay): id, type, all properties. "Open in HQL" button pre-fills the HQL Editor with `V <id> | RETURN *`.
- **Controls:** zoom in/out, fit-to-canvas, reset layout.
- Results are parsed from the `/__hql_runtime_eval` response: any object with an `id` and no `from`/`to` is treated as a node; any object with `from` and `to` is treated as an edge.

### 3.5 View: Diagnostics

Auto-refresh toggle (default: off, 10s interval when on). Two panels:

- **System stats** — from `POST /diagnostics`: node count, edge count, DB size on disk, uptime.
- **HNSW health** — from `POST /hnsw_health`: status (healthy/degraded), vector count, soft-deleted count. Integrity check button triggers `POST /hnsw_integrity` (slow — on demand only).

No token management UI — that belongs in a future expansion once `docs/superpowers/plans/2026-05-22-auth-and-rbac.md` ships.

### 3.6 View: Vectors

Three operation cards:

- **Delete by ID:** text input (UUID or u128 string) + Soft Delete button + Hard Delete button. Calls `POST /vector_soft_delete` or `POST /vector_hard_delete` with `{ "id": "<value>" }`. Requires ReadWrite role when auth is enabled.
- **Purge soft-deleted:** button with confirmation prompt. Calls `POST /purge_soft_deleted`. Requires ReadWrite.
- **Rebuild index:** button with confirmation prompt ("This rebuilds the entire HNSW index and may take a moment"). Calls `POST /rebuild_vector_index`. Requires ReadWrite.

### 3.7 API client (`api/client.ts`)

Single `SparrowClient` class. Every method reads `baseUrl` and `apiKey` from the connection store and passes `x-api-key` on every request (empty string when no auth). This is forward-compatible with the auth/RBAC plan — when the other team ships token enforcement, the Studio works without changes.

```ts
class SparrowClient {
  async hqlEval(query: string): Promise<unknown>
  async introspect(): Promise<SchemaResponse>
  async nodeDetails(id: string): Promise<unknown>
  async diagnostics(): Promise<DiagnosticsResponse>
  async hnswHealth(): Promise<HnswHealthResponse>
  async hnswIntegrity(): Promise<HnswIntegrityResponse>
  async vectorSoftDelete(id: string): Promise<void>
  async vectorHardDelete(id: string): Promise<void>
  async purgeSoftDeleted(): Promise<void>
  async rebuildVectorIndex(): Promise<void>
}
```

---

## 4. Build Pipeline

### Dev workflow

```bash
# Terminal 1 — Sparrow running normally
sparrow start <instance>

# Terminal 2 — Frontend hot reload (Vite proxies API calls to :2048)
pnpm --filter @sparrowdb/studio dev
# opens http://localhost:5173
```

No Rust rebuild needed during frontend development.

### Production build

```bash
# Step 1: build frontend
pnpm --filter @sparrowdb/studio build
# → packages/studio/dist/

# Step 2: Rust embeds the assets automatically
cargo build --package sparrow-container
# rust-embed reads packages/studio/dist/ at compile time
```

### CI (GitHub Actions)

```yaml
- name: Build Studio frontend
  run: pnpm --filter @sparrowdb/studio build

- name: Build container (embeds Studio)
  run: cargo build --package sparrow-container
```

Frontend build must run before the Rust build. `build.rs` emits `cargo:rerun-if-changed=../../packages/studio/dist/` so incremental Rust builds re-embed when the frontend changes.

### `.gitignore` additions

```
packages/studio/dist/
packages/studio/node_modules/
.superpowers/
```

---

## 5. Dependencies

### Frontend (`packages/studio/package.json`)

| Package | Purpose |
|---------|---------|
| `solid-js` | UI framework |
| `vite` + `vite-plugin-solid` | Build tooling + dev server |
| `@codemirror/state`, `@codemirror/view`, `@codemirror/language` | Editor core |
| `@codemirror/lang-sql` (as starting grammar reference) | Basis for HQL grammar |
| `cytoscape` | Graph canvas |
| `cytoscape-cose-bilkent` | Graph layout algorithm |

No UI component library — plain CSS in keeping with the custom vanilla CSS approach used in helixdb-explorer. Dark theme only.

### Rust (`crates/sparrow-studio/Cargo.toml`)

| Crate | Purpose |
|-------|---------|
| `rust-embed` | Compile-time asset embedding |
| `axum` | Route handler (same version as sparrow-core) |
| `mime_guess` | Content-type from file extension |

---

## 6. Auth / RBAC Integration

The Studio is designed to be forward-compatible with `docs/superpowers/plans/2026-05-22-auth-and-rbac.md`:

- **Static files (`/__studio/*`) are never auth-gated** — the UI shell loads regardless.
- **The API client always sends `x-api-key`** — empty string in dev mode (auth disabled), populated from `localStorage` when the user sets a token. No Studio code changes needed when auth ships.
- **`__hql_runtime_eval` is registered as a write route** — when studio is compiled in, the env gate is bypassed and the route is added to `write_routes` in `SparrowRouter`. This ensures write-role enforcement from the auth plan applies correctly.
- **No token management UI in v1** — planned as a Diagnostics extension post-auth-plan completion.

---

## 7. What Is Not Built

- Schema editing / migrations (separate roadmap item)
- Multi-connection support (single instance only)
- Query history persistence beyond `localStorage`
- Vector similarity search UI (search by text/vector — the backend endpoints exist, deferred to v2)
- Light theme
- Token management panel (after auth plan ships)
