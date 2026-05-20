# TypeScript Query Client Generation Plan (v1)

## Goal

Implement opt-in TypeScript artifact generation for Helix queries at compile/build time.

The v1 deliverable is a generated TypeScript file that exports:

- Typed input types per query.
- Typed response types per query.
- Typed wrapper functions per query that call `helix-ts`.

This work is intentionally compile-time only in v1 (no runtime generation from `/introspect`).

---

## Why This Work

Today:

- HQL is compiled ahead-of-time into Rust via `helixc`.
- Rust handlers are generated into `queries.rs` and built into the DB binary.
- `helix-ts` currently exposes a generic `query(endpoint, data)` API with untyped request/response shapes.

As a result, users do not get per-query TypeScript safety by default. This plan adds that capability without changing runtime query behavior.

---

## Current Architecture Context

### Compilation and Generation Flow

- Parse: `HelixParser::parse_source(...)`
- Analyze: `analyze(&source)`
- Generate Rust: `helixc::generator::generate(...)` -> writes `queries.rs`

Main integration points:

- `helix-db/src/helixc/generator/mod.rs`
- `helix-cli/src/commands/compile.rs`
- `helix-cli/src/commands/build.rs`
- `helix-cli/src/commands/push.rs`
- `helix-cli/src/utils.rs` (`helixc_utils` module)

### Metadata Already Available

- Query input parameter metadata already exists in analyzer output (`GeneratedQuery.parameters`, `sub_parameters`).
- Rich return metadata already exists for struct-based return generation (`return_structs`, `field_infos`, nested traversal metadata).
- There is also a legacy return path (`use_struct_returns = false`) for complex object literal returns.

### Runtime / SDK Context

- Query request shape is `POST /{queryName}` with request body = query input JSON.
- `helix-ts` currently defines:
  - `HelixDBClient.query(endpoint: string, data: Record<string, any>): Promise<Record<string, any>>`

---

## Decisions Locked for v1

1. **Source of truth**: compile-time only (from `helixc` output).
2. **Artifacts**: generate both types and typed wrapper functions.
3. **Wrapper target**: generated wrappers call `helix-ts` base client.

---

## Scope and Non-Goals

### In Scope

- Add compile-time TS generation in `helixc`.
- Add CLI flags to enable TS generation during `compile`, `build`, and `push`.
- Emit deterministic `queries.generated.ts` output.
- Cover major return classes: primitive, collection, nested struct returns, and aggregate/group-by shapes.
- Add tests and docs for the new behavior.

### Out of Scope (v1)

- Generating clients from `/introspect`.
- Replacing or redesigning `helix-ts` transport behavior.
- Changing query runtime semantics.
- Guaranteeing perfect static typing for all legacy object-literal fallback cases (fallback type strategy will be used where needed).

---

## Proposed v1 Artifact Contract

Default generated file name:

- `queries.generated.ts`

Contents:

1. Shared aliases/helpers (for stable generated types).
2. Query-specific input types.
3. Query-specific response types.
4. Typed wrapper functions that call `client.query("<queryName>", input)`.

Wrapper signature pattern:

```ts
import type { HelixDBClient } from "helix-ts";

export type GetUserInput = { user_id: string };
export type GetUserResponse = { user: GetUserUserReturnType };

export async function getUser(
  client: HelixDBClient,
  input: GetUserInput,
): Promise<GetUserResponse> {
  return client.query("getUser", input) as Promise<GetUserResponse>;
}
```

---

## Type Mapping Strategy

### Input Type Mapping

From HQL `FieldType` / generated input parameter metadata:

- `String` -> `string`
- `Boolean` -> `boolean`
- Numeric scalar types (`I*`, `U*`, `F*`) -> `number`
- `ID` (`Uuid`) -> `string`
- `Date` -> `string` (RFC3339 payload)
- `Array<T>` -> `T[]`
- `Object` -> nested generated TS object type
- Optional parameter -> optional property (`name?: ...`)
- Identifier schema references -> generated schema-like input type where possible; otherwise named reference fallback type alias

### Response Type Mapping

From `ReturnValueStruct`, `ReturnFieldInfo`, and return metadata:

- Primitive returns:
  - string-like -> `string`
  - boolean -> `boolean`
  - numeric -> `number`
- `Value`-style fields -> `HelixValue` alias
- Optional property values -> `HelixValue | null`
- `TraversalValue`-style fields -> `HelixTraversalValue` alias
- `Vec<T>` / arrays -> `T[]`
- Nested returns -> generated nested TS interfaces/types
- Aggregate/group-by returns -> explicit serialized response shape aliases based on existing runtime serialization behavior
- Legacy object-literal fallback path (`use_struct_returns = false`) -> safe fallback typing (`HelixValue` or `unknown`) when exact shape cannot be guaranteed from metadata

---

## Implementation Workstreams

## 1) Add TS Generator Module in `helix-db`

### Files

- `helix-db/src/helixc/generator/mod.rs`
- `helix-db/src/helixc/generator/ts_client.rs` (new)

### Changes

- Add a dedicated TS renderer (`render_ts_client`) that takes analyzed generator `Source` and returns deterministic TypeScript text.
- Add a writer method to emit `queries.generated.ts`.
- Keep Rust `queries.rs` generation unchanged by default.
- Ensure generation order is deterministic (stable sort where needed).

### Notes

- Prefer adding a pure render function for testability.
- Avoid relying on formatting/parsing Rust type strings for TS typing whenever structured metadata exists.

---

## 2) Strengthen Primitive Return Typing Metadata

### Files

- `helix-db/src/helixc/generator/return_values.rs`
- `helix-db/src/helixc/analyzer/methods/query_validation.rs`
- `helix-db/src/helixc/generator/queries.rs`

### Changes

- Extend primitive return metadata so TS generation can infer exact scalar type class (string/number/boolean) without string heuristics.
- Populate metadata in analyzer at the point primitive return structs are built.
- Preserve existing runtime behavior and Rust codegen output.

### Notes

- This is a compatibility-safe metadata extension for codegen; do not alter query semantics.

---

## 3) CLI Flag and Flow Integration

### Files

- `helix-cli/src/main.rs`
- `helix-cli/src/commands/compile.rs`
- `helix-cli/src/commands/build.rs`
- `helix-cli/src/commands/push.rs`
- `helix-cli/src/utils.rs` (within `helixc_utils`)

### Flag design

- `--ts-client` (boolean): enable TypeScript artifact generation.
- `--ts-output <path>` (optional): explicit output path for generated TS file.

### Behavior

- `compile`:
  - if `--ts-client` set, emit `queries.generated.ts` into compile output directory (or `--ts-output`).
- `build` and `push`:
  - if `--ts-client` set, emit `queries.generated.ts` to project root by default (or `--ts-output`).
- When flag is absent, behavior remains exactly as today.

### Path handling

- Absolute `--ts-output`: use directly.
- Relative `--ts-output`: resolve from project root.
- Ensure parent directories exist before write.

---

## 4) Runtime Contract Alignment with `helix-ts`

### Constraint

- Generated wrappers should depend only on stable, public `helix-ts` surface (`HelixDBClient.query(...)`).

### Expected output import style

- `import type { HelixDBClient } from "helix-ts";`

### Notes

- v1 does not require changing `helix-ts` runtime behavior.
- If future `helix-ts` adds a generic query primitive, wrappers can switch with minimal template changes.

---

## 5) Test Coverage

## `helix-db` unit tests

Add/extend tests around generator rendering:

- primitive input/output mapping
- optional params
- nested object params
- nested return structures
- aggregate/group-by return typing
- legacy fallback typing behavior

## `helix-cli` tests

Add tests to `helix-cli/src/tests/compile_tests.rs`:

- `helix compile --ts-client` creates `queries.generated.ts`
- `helix compile --ts-client --ts-output <path>` writes custom path
- generated TS file contains expected exports/wrapper function names

Add similar tests for build path integration only where practical; avoid network-heavy dependency in non-ignored tests.

---

## 6) Documentation

### Files

- `helix-cli/README.md`
- `README.md` (root)

### Updates

- Add `--ts-client` and `--ts-output` examples.
- Add a quick usage snippet showing import and call of generated wrappers.
- Clarify that TS generation is optional and compile-time driven.

---

## Risks and Mitigations

### Risk: return type complexity in legacy object-literal paths

Mitigation:

- Emit conservative fallback (`HelixValue` or `unknown`) where exact static shape cannot be inferred safely.
- Document this limitation in generated header and README.

### Risk: drift between Rust runtime serialization and TS assumptions

Mitigation:

- Base mapping on existing analyzer/generator structured metadata.
- Add snapshot-style tests for representative query shapes.

### Risk: CLI behavior regressions

Mitigation:

- Keep TS generation fully opt-in.
- Preserve existing code paths when flags are absent.

---

## Rollout Plan

1. Land `helix-db` TS renderer + metadata updates.
2. Land CLI flags and wiring.
3. Land tests.
4. Land docs.

Optional follow-up (future): introspect-based external client generation command.

---

## Definition of Done

Feature is done when:

- `helix compile --ts-client` emits `queries.generated.ts` with typed inputs, outputs, and wrappers.
- `helix build --ts-client` and `helix push --ts-client` can also emit TS artifacts.
- Existing compile/build/push behavior is unchanged when `--ts-client` is not set.
- Tests cover core mapping and CLI generation paths.
- Documentation includes end-to-end usage.

---

## Implementation Checklist

### Generator Core

- [ ] Add `helix-db/src/helixc/generator/ts_client.rs`.
- [ ] Implement pure renderer `render_ts_client(&Source) -> String`.
- [ ] Implement writer for `queries.generated.ts` output.
- [ ] Register module + write path in `helix-db/src/helixc/generator/mod.rs`.
- [ ] Keep deterministic output ordering.

### Return Metadata

- [ ] Extend primitive return metadata in `return_values.rs`.
- [ ] Populate primitive metadata during analysis in `query_validation.rs`.
- [ ] Ensure generator defaults/constructors remain consistent in `queries.rs`.

### CLI Flags and Wiring

- [ ] Add `--ts-client` and `--ts-output` to `compile` command in `main.rs` and `commands/compile.rs`.
- [ ] Add `--ts-client` and `--ts-output` to `build` command in `main.rs` and `commands/build.rs`.
- [ ] Add `--ts-client` and `--ts-output` to `push` command in `main.rs` and `commands/push.rs`.
- [ ] Add generation helper plumbing in `utils.rs` (`helixc_utils`).
- [ ] Implement output path resolution + parent dir creation.

### Generated Output Contract

- [ ] Emit shared helper aliases (`HelixValue`, `HelixTraversalValue`, etc.).
- [ ] Emit query input types.
- [ ] Emit query response types.
- [ ] Emit typed wrapper functions using `HelixDBClient`.
- [ ] Validate wrapper endpoint names match query names exactly.

### Tests

- [ ] Add `helix-db` tests for TS primitive mappings.
- [ ] Add `helix-db` tests for nested return mappings.
- [ ] Add `helix-db` tests for aggregate/group-by response mappings.
- [ ] Add `helix-db` tests for legacy fallback typing behavior.
- [ ] Add `helix-cli` compile test for default TS output generation.
- [ ] Add `helix-cli` compile test for custom TS output path.

### Docs

- [ ] Update `helix-cli/README.md` with new flags and examples.
- [ ] Update root `README.md` with generated TS wrapper usage snippet.

### Verification

- [ ] `cargo check -p helix-db`
- [ ] `cargo check -p helix-cli`
- [ ] `cargo test -p helix-db`
- [ ] `cargo test -p helix-cli compile_tests`
- [ ] Manual smoke: run `helix compile --ts-client` in a sample project and verify emitted file.
