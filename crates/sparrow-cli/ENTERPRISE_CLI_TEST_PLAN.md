# Helix CLI Enterprise Cluster Testing Plan

## Status

- [x] Initial plan drafted
- [x] Execution started
- [ ] All critical blockers resolved
- [ ] All automated tests added and passing
- [ ] Staging E2E completed
- [ ] Sign-off completed

## Scope

This plan covers end-to-end testing for **Helix CLI + enterprise clusters**, focused on:

1. Pushing new `helix_dsl` Rust queries via CLI after generating `queries.json`
2. Running `helix sync` for enterprise clusters
3. Verifying the workspace-type rule: **only `enterprise` workspaces can use enterprise clusters**

Out of scope for now: non-enterprise cluster lifecycle details, logs UX polish, and unrelated Fly/ECR flows.

## Repos and Components Under Test

- CLI: `~/GitHub/helix-db/helix-cli`
- Dashboard backend/API: `~/GitHub/helix-cloud-build/build-gateway`
- Enterprise runtime/provisioner: `~/GitHub/helix-hyperscale`
- Enterprise query DSL generator: `~/GitHub/helix-enterprise-ql`

## Critical Risks Identified (Test First)

- [x] Verify/fix enterprise cluster project endpoint contract (`/api/cli/enterprise-clusters/{cluster_id}/project`) used by CLI sync post-reconcile
  - Staging now returns `200` and CLI post-sync metadata refresh succeeds.
- [ ] Verify CLI node-type values are accepted by backend create-enterprise-cluster API
- [ ] Verify enterprise create constraints (HA/min instances) are aligned across CLI prompts and backend validation
- [ ] Verify enterprise deploy/sync enforce workspace-type invariants (enterprise-only)
- [ ] Verify runtime query routing dependencies (`introspect.json` vs query bundle path) do not break post-push query execution

---

## Phase 0 - Environment and Fixtures

### 0.1 Accounts and Auth

- [ ] Confirm test user can run `helix auth login`
- [x] Confirm API key is present in `~/.helix/credentials`
- [x] Confirm user has access to at least one enterprise workspace
- [x] Confirm user has access to at least one non-enterprise workspace

### 0.2 Workspace/Project/Cluster Fixtures

- [x] Create or identify enterprise workspace fixture (`WS_ENT`)
- [x] Create or identify organization/personal workspace fixture (`WS_STD`)
- [x] Create project in enterprise workspace (`PROJ_ENT`)
- [x] Create enterprise cluster in enterprise workspace (`CLUSTER_ENT`)
- [ ] Create standard cluster in non-enterprise workspace (`CLUSTER_STD`)
  - Blocked: personal workspace project creation returned `402 Billing setup is required before creating projects`.

### 0.3 Query Project Fixtures

- [x] Prepare valid enterprise query project (Cargo.toml + `src/*.rs` + `helix_dsl::generate()` in main)
- [ ] Prepare invalid compile fixture (Rust compile failure)
- [ ] Prepare missing/empty `queries.json` fixture
- [ ] Prepare source snapshot fixture with nested paths + allowed files
- [x] Prepare source snapshot fixture with disallowed/unsafe paths for rejection tests

---

## Phase 1 - Workspace Type and Access Control

### 1.1 Enterprise Workspace Gating (Creation)

- [ ] `helix add <name> --cloud` in enterprise workspace allows enterprise cluster creation path
- [ ] `helix add <name> --cloud` in non-enterprise workspace forces standard cluster path
- [ ] Direct API test: creating enterprise cluster from non-enterprise workspace returns `403`
- [ ] Error text clearly states enterprise workspace requirement

### 1.2 Cluster Access Control (Deploy/Sync)

- [x] Enterprise deploy endpoint rejects user without cluster access (`403`)
- [x] Enterprise sync endpoint rejects user without cluster access (`403`)
  - Verified with a valid API key against an unowned cluster UUID; both endpoints returned `403 Access denied`.
- [x] Enterprise deploy endpoint rejects invalid API key (`401`/auth failure path)
- [x] Enterprise sync endpoint rejects invalid API key (`401`/auth failure path)

### 1.3 Workspace-Type Invariant Beyond Create

- [ ] Attempt enterprise deploy against cluster tied to non-enterprise workspace is blocked
- [ ] Attempt enterprise sync against cluster tied to non-enterprise workspace is blocked
- [ ] Access-control behavior is consistent across dashboard auth and CLI API-key auth paths

---

## Phase 2 - Enterprise Push Path (`helix push`)

### 2.1 Local Generation + Validation

- [x] Valid query project runs `cargo run --manifest-path <queries_dir>/Cargo.toml` and generates non-empty `queries.json`
- [ ] Duplicate registered query names fail generation with clear error
- [ ] Unsupported/invalid macro parameter types fail at compile time as expected
- [ ] Missing `Cargo.toml` fails push with actionable message
- [ ] Missing `queries.json` after compile fails push with actionable message
- [ ] Empty `queries.json` fails push with actionable message

### 2.2 Snapshot Upload and Sanitization

- [x] Push uploads `queries.json` to canonical key: `{workspace_id}/{project_id}/{cluster_id}/queries.json`
- [x] Push uploads source snapshot allowlist only (`Cargo.toml`, `Cargo.lock`, `build.rs`, `rust-toolchain*`, `src/**`, `.cargo/*.toml`)
- [x] Push excludes `queries.json` from source snapshot file map
- [x] Push rejects unsafe source paths (`..`, absolute paths)
- [x] Push clears previous source prefix before uploading new snapshot
- [x] Push preserves optional `helix.toml` upload behavior

### 2.3 Reconcile Trigger

- [ ] Push triggers provisioner reconcile with `QUERY_BUNDLE_S3_PATH` helm value
- [x] Push returns accepted/success response with key and size metadata
- [ ] Provisioner upsert errors surface clearly to CLI

### 2.4 Runtime Verification After Push

- [ ] Enterprise runtime can load new query bundle from configured object storage path
- [ ] Gateway query endpoint can execute at least one newly pushed read query
- [ ] Gateway query endpoint can execute at least one newly pushed write query
- [ ] Route rename/removal behavior is validated after second push

---

## Phase 3 - Enterprise Sync Path (`helix sync`)

### 3.1 Sync API Contract

- [x] Enterprise sync response includes `source_files`
- [x] Enterprise sync response includes `file_metadata.sha256`
- [x] Enterprise sync response includes `file_metadata.last_modified_ms`
- [x] Enterprise sync returns `helix_toml` when present
- [ ] Enterprise sync handles missing remote snapshot with expected fallback behavior

### 3.2 Local Reconciliation Scenarios

- [x] Local empty + remote populated => pull flow writes files locally
- [x] Local populated + remote empty => push prompt/flow works correctly
- [x] Diverged with local newer => push option available when local validates
- [x] Diverged with remote newer => pull option works
- [x] Tie/unknown timestamps => safe default no-op in non-interactive/`--yes`
- [x] Pull flow removes local files not present remotely

### 3.3 Regeneration and Safety

- [x] After enterprise pull, CLI regenerates `queries.json`
- [ ] Sync only writes sanitized relative paths
- [ ] Sync ignores unsafe remote paths and non-allowlisted files
- [x] Sync skips non-UTF8 source files safely without crashing

### 3.4 Project Metadata Refresh

- [x] `helix sync <enterprise-instance>` successfully refreshes `helix.toml` project/cluster metadata
- [x] CLI enterprise cluster-to-project lookup endpoint path is valid and reachable
- [x] No post-sync failure due to missing enterprise project metadata route

---

## Phase 4 - Failure, Recovery, and Regression

### 4.1 Failure Injection

- [ ] Simulate S3 upload failure during enterprise push and verify clear CLI/backend errors
- [ ] Simulate provisioner failure after upload and verify non-silent failure handling
- [ ] Simulate transient cloud/API failure during sync and verify retry/fallback behavior where expected

### 4.2 Scale/Limit Tests

- [ ] Source snapshot near max file count limit behaves as expected
- [ ] Source snapshot over max file count limit is rejected with clear error
- [ ] Source snapshot near max byte limit behaves as expected
- [ ] Source snapshot over max byte limit is rejected with clear error

### 4.3 Contract Drift Guards

- [ ] Add route/contract test to catch missing CLI enterprise endpoints
- [ ] Add compatibility test for query bundle version decode path
- [ ] Add CI check that enterprise sync/deploy response schema matches CLI expectation

---

## Automation Checklist by Repo

### `helix-db/helix-cli`

- [ ] Add unit tests for enterprise source allowlist/path sanitization (if gaps remain)
- [ ] Add tests for enterprise sync decision matrix (local-only/remote-only/diverged/tie)
- [ ] Add tests for enterprise compile/regenerate failure messages
- [ ] Add tests for enterprise deploy-by-cluster-id behavior with stale helix.toml mapping

### `helix-cloud-build/build-gateway`

- [ ] Add handler tests for CLI enterprise deploy endpoint (base64, empty payload, unsafe paths)
- [ ] Add handler tests for CLI enterprise sync endpoint (authz, metadata shape, helix.toml retrieval)
- [ ] Add tests for enterprise workspace-type enforcement on create + deploy + sync
- [ ] Add route registration test for all CLI enterprise endpoints used by CLI

### `helix-hyperscale`

- [ ] Add tests for query-bundle loading via env (`PATH_TO_QUERIES`, `QUERY_BUNDLE_S3_PATH`)
- [ ] Add tests for decode failure path (invalid bundle bytes)
- [ ] Add integration test to verify runtime query routing after bundle refresh

### `helix-enterprise-ql`

- [ ] Add tests for generate/serialize/deserialize compatibility with runtime expectations
- [ ] Add tests for duplicate query registration failure
- [ ] Add tests for version mismatch handling in query bundle

---

## Staging E2E Runbook Checklist

### Pre-Run

- [ ] Confirm all required env vars/secrets for staging are present
- [x] Confirm enterprise cluster is running and reachable
- [ ] Confirm test query project path and clean git state in fixture repo

### Run

- [x] Generate query bundle (`cargo run --manifest-path <queries_dir>/Cargo.toml`)
- [x] Run `helix push <enterprise-instance>`
- [ ] Verify bundle + source snapshot objects in S3 prefix
- [ ] Verify provisioner reconcile request accepted and cluster healthy
- [ ] Execute smoke read query through gateway
- [ ] Execute smoke write query through gateway
- [x] Run `helix sync <enterprise-instance>` and verify local files
- [x] Validate deletions/renames propagate correctly after second push+sync cycle

### Post-Run

- [ ] Archive logs (CLI output, backend logs, provisioner logs)
- [ ] Record pass/fail by test case and root-cause failures
- [ ] Create follow-up issues with severity and owner

---

## Exit Criteria (Definition of Done)

- [ ] Enterprise push is reliable: compile -> generate -> upload -> reconcile -> query execution
- [ ] Enterprise sync is reliable: metadata-aware reconciliation with safe path handling
- [ ] Workspace-type rule is enforced end-to-end for enterprise lifecycle paths
- [ ] No known CLI/backend route or payload contract drift for enterprise push/sync
- [ ] CI includes the new automated coverage for critical enterprise paths

---

## Execution Notes

- Tick checkboxes in this file directly as work progresses.
- When a checkbox fails, add a short note under it or link to the issue/PR.
- Keep this file as the source of truth for enterprise CLI testing progress.
