# Runtime HQL Interpreter Plan

## Locked Scope

- Runtime requests must contain a full `QUERY ... => ... RETURN ...` block.
- Runtime path must support all language features, including writes.
- Runtime endpoint is server-side and gated by env var (`HELIX_RUNTIME_HQL`, default disabled).
- Type checking remains mandatory via parser + analyzer before execution.
- Deployed schema is not sent per request; raw HQL schema is stored once and reused.

## Architecture

- Keep AOT compile/deploy flow unchanged; add a parallel runtime-eval flow for fast iteration.
- Add a dedicated route: `POST /__hql_runtime_eval`.
  - Route name must remain a single path segment because request parsing rejects names with `/` (`helix-db/src/protocol/request.rs`).
- Gate route registration in `helix-container/src/main.rs` with `HELIX_RUNTIME_HQL`.
  - If disabled, do not register route (clean `404`).
- Keep existing `schema` introspection JSON behavior unchanged (`helix-db/src/helix_engine/traversal_core/config.rs`).
- Add a new config/storage field for raw deployed schema:
  - `hql_schema_raw: Option<String>` in `Config`
  - `hql_schema_raw: Option<String>` in `StorageConfig`
- Always treat runtime eval route as a write route so worker routing is safe for mutating queries (`helix-db/src/helix_gateway/worker_pool/mod.rs`).

## Execution Model

- Runtime request body contains only:
  - `query: String`
  - `params: Option<Map<String, Value>>`
- Runtime flow:
  1. Read deployed schema from `hql_schema_raw`.
  2. Parse schema once and cache by schema hash.
  3. Parse incoming runtime query source.
  4. Enforce request shape:
     - exactly one query
     - no schema defs
     - no migration defs
  5. Merge cached schema AST + request query AST into a single `Source`.
  6. Run analyzer for full type checking.
  7. Lower validated result into runtime semantic plan (non-string IR).
  8. Execute against existing traversal/storage primitives.
  9. Return in existing protocol formats (`in_fmt`/`out_fmt`).

## Phased Implementation Plan

### PR1 - Config and Schema Persistence

- Add `hql_schema_raw` plumbing through config and storage.
- Embed raw deployed HQL schema in generated config.
- Add schema hash support for cache invalidation.

### PR2 - Route, Env Gate, and DTO Scaffolding

- Add env parsing for `HELIX_RUNTIME_HQL` in container startup.
- Register `__hql_runtime_eval` route only when enabled.
- Mark runtime route as write route.
- Add request/response DTO skeletons.

### PR3 - Parse/Analyze Runtime Validation

- Implement runtime parser/analyzer bridge using cached deployed schema.
- Enforce full query block requirement and request shape checks.
- Add structured diagnostics mapping (parse + analyzer).

### PR4 - Runtime Plan and Core Executor

- Introduce runtime semantic plan IR.
- Lower validated queries into runtime plan.
- Execute core statements/traversals/returns/writes.

### PR5 - Full Feature Parity

- Cover nested closures/remaps/computed expressions.
- Cover vector + BM25 + rerank + shortest path features.
- Ensure behavior parity with compiled path.

### PR6 - Async Embed and Error Surface

- Add async embedding continuation parity.
- Finalize status-code/error mapping and protocol consistency.

### PR7 - Conformance, Hardening, and Docs

- Differential tests (compiled vs runtime) across full feature matrix.
- Concurrency/stability tests.
- Rollout docs + guardrails.

## Implementation Checklist

### Config + Schema Persistence

- [ ] Add `hql_schema_raw: Option<String>` to `Config` in `helix-db/src/helix_engine/traversal_core/config.rs`.
- [ ] Add `hql_schema_raw: Option<String>` to `StorageConfig` in `helix-db/src/helix_engine/storage_core/mod.rs`.
- [ ] Wire config-to-storage propagation in `HelixGraphStorage::new`.
- [ ] Update generated `config()` output to include `hql_schema_raw`.
- [ ] Generate raw schema text from parsed schema/version order in compiler path.
- [ ] Keep current `schema` introspection JSON output unchanged.

### Route + Env Gate + Wiring

- [ ] Add `HELIX_RUNTIME_HQL` env parsing in `helix-container/src/main.rs`.
- [ ] Register runtime route only when env flag is enabled.
- [ ] Reserve route name `__hql_runtime_eval` and add collision guard.
- [ ] Insert runtime handler into `query_routes` map.
- [ ] Insert runtime route into `write_routes` set.
- [ ] Add route-gating tests (`disabled -> 404`, `enabled -> reachable`).

### Runtime Request Contract

- [ ] Add runtime request DTO with `query` + `params`.
- [ ] Enforce full query block and exactly one `QUERY`.
- [ ] Reject schema/migration blocks in runtime payload.
- [ ] Return clear 4xx/422 for malformed input.
- [ ] Validate parameter presence, optionality, and type compatibility.
- [ ] Enforce unknown-parameter policy (recommended: reject unknown keys).

### Schema Cache

- [ ] Implement parsed-schema cache keyed by `schema_hash`.
- [ ] Parse `hql_schema_raw` once per hash and reuse.
- [ ] Invalidate cache when schema hash changes.
- [ ] Handle missing/unparseable schema with explicit errors.
- [ ] Add metrics/logging for cache hit/miss and parse failures.
- [ ] Add isolation tests for multi-instance/test scenarios.

### Analyzer + Diagnostics

- [ ] Build merged `Source` (cached schema + request query).
- [ ] Run analyzer before execution.
- [ ] Map parser/analyzer failures to structured diagnostics DTO.
- [ ] Include line/column/message/code/hint in diagnostics.
- [ ] Return validation failures as 4xx/422, not generic 500.
- [ ] Keep non-runtime route behavior unchanged.

### Runtime Plan + Lowering

- [ ] Add runtime semantic plan module (no code-string execution).
- [ ] Lower statements: assignment, traversal expression, drop, for-loop.
- [ ] Lower traversal starts: N/E/V, by id, by index, anonymous/id traversal.
- [ ] Lower traversal ops: Out/In/OutE/InE/FromN/ToN/FromV/ToV/Intersect/Where/Range/Order/Count/FIRST.
- [ ] Lower mutation ops: AddN/AddE/AddV/BatchAddV/Update/UpsertN/UpsertE/UpsertV/Drop.
- [ ] Lower vector/search ops: SearchV/SearchBM25 + rerank steps.
- [ ] Lower path ops: shortest path variants.
- [ ] Lower return projections: object remap, nested traversal fields, arrays, computed expressions, closures.
- [ ] Lower boolean/math expression trees with analyzer-validated typing.

### Executor

- [ ] Add runtime execution context (graph, txn, arena, vars, params).
- [ ] Execute statements in-order with proper scope semantics.
- [ ] Open read/write txn based on plan mutability (route still sent to writer worker).
- [ ] Execute traversal plan using existing traversal adapters/util ops.
- [ ] Implement for-loop destructuring/object access parity.
- [ ] Implement return materialization parity with compiled path.
- [ ] Ensure commit/rollback semantics for mutations match compiled handlers.
- [ ] Support async continuation flow for embedding operations.

### Error and Protocol Surface

- [ ] Add explicit bad-request/validation error variants to runtime mapping.
- [ ] Keep runtime errors structured and machine-readable.
- [ ] Include query name + diagnostic context in logs.
- [ ] Preserve Accept/Content-Type behavior from existing protocol path.
- [ ] Add tests for status correctness (400/404/422/500).
- [ ] Ensure clear errors for disabled route vs missing schema vs type mismatch.

### Testing and Conformance

- [ ] Unit tests for schema extraction + cache behavior.
- [ ] Unit tests for param coercion and optional parameter handling.
- [ ] Unit tests for lowering coverage across statement/traversal kinds.
- [ ] Integration tests for env-gated route + write-worker routing.
- [ ] Feature-matrix tests across grammar families in `helix-db/src/grammar.pest`.
- [ ] Differential tests: compiled vs runtime output parity.
- [ ] Differential mutation tests: identical post-state in storage.
- [ ] Concurrency/stress tests for simultaneous runtime requests.
- [ ] Regression fixtures for closures/nested remaps/rerank/path edge cases.

### Docs and Rollout

- [ ] Document `HELIX_RUNTIME_HQL` (default disabled).
- [ ] Document runtime request format and examples.
- [ ] Document full-query-only requirement and no-inline-schema rule.
- [ ] Add troubleshooting guide for diagnostics and common failures.
- [ ] Add explicit guidance: runtime eval is intended for dev workflows.

## Definition of Done

- Runtime eval executes full HQL surface, including writes, without redeploy.
- Type checking remains mandatory and semantically aligned with compiler analyzer.
- Runtime path uses deployed `hql_schema_raw`, not per-request schema payloads.
- Endpoint is disabled by default and only active when `HELIX_RUNTIME_HQL=true`.
- Conformance suite confirms compiled vs runtime parity for outputs and mutations.
