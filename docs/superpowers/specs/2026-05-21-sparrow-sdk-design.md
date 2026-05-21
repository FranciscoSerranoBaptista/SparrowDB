# sparrow-sdk: Native SDK for SparrowDB

**Date:** 2026-05-21
**Status:** Approved

## Overview

Move and rebrand the upstream HelixDB Rust SDK into SparrowDB as a first-party crate named `sparrow-sdk`. Deliver a README and an LLM-friendly reference file alongside the code changes.

## Scope

Three deliverables:

1. **Crate rename** — `sdks/rust/` becomes `sparrow-sdk/` at the repo root; all Helix/helix branding removed from source
2. **README** — `sparrow-sdk/README.md`
3. **LLM reference** — `sparrow-sdk/sparrowdb-sdk-llm.txt`

## 1. Rename / Move

### Location

`sdks/rust/` → `sparrow-sdk/` at the repository root, consistent with the existing `sparrow-db/`, `sparrow-cli/`, `sparrow-memory/` layout.

Add `"sparrow-sdk"` to the root `Cargo.toml` workspace `members`.

### Symbol renames

| Old | New |
|-----|-----|
| crate name `helix-db` | `sparrow-sdk` |
| `helix_db::` (all paths in source + docs) | `sparrow_sdk::` |
| `HelixError` | `SparrowError` |
| `HelixDBClient` (type alias) | `SparrowDBClient` |
| `extern crate self as helix_db` | `extern crate self as sparrow_sdk` |
| All doc comments mentioning "Helix" / "HelixDB" | "SparrowDB" |
| `Cargo.toml` description, keywords, repository | Updated to SparrowDB |

### Wire format — unchanged

The HTTP header strings sent by the SDK (`x-helix-require-writer`, `x-helix-warm`, `x-helix-await-durable`) are server-defined protocol values. They are preserved as-is in the string literals. Only the Rust method/constant names that set them change (e.g. `.writer_only()` remains `.writer_only()` — the name is already neutral).

### Macro dependency

`helix-dsl-macros` provides the `#[register]` proc-macro. The upstream `Cargo.toml` references it via both a `path` (local subdirectory that was not copied) and a crates.io version. Resolution: drop the `path` override, keep `helix-dsl-macros = "0.2.0"` from crates.io. Porting the macro into `sparrow-macros` is deferred to a future task.

## 2. README (`sparrow-sdk/README.md`)

Sections in order:

1. What it is (one paragraph)
2. Install (`Cargo.toml` snippet)
3. Quick start (Client construction + one complete round-trip)
4. Core shape (`read_batch` / `write_batch` pattern)
5. Read batches (3 examples: basic traversal, filter+sort+project, parameterised)
6. Conditional queries (`var_as_if` + `BatchCondition`)
7. Write batches (node/edge creation, property mutation)
8. Executing queries (`Client`, `QueryBuilder`, header toggles)
9. Registered queries (`#[register]` end-to-end)
10. Vector search (index creation, node search, edge search, multitenancy)
11. Edge-first reads
12. Traversal reference (lookup table of all DSL methods by category)
13. Error handling (`SparrowError` variants)
14. License

## 3. LLM Reference (`sparrow-sdk/sparrowdb-sdk-llm.txt`)

A plain-text file optimised for pasting into an LLM context. Sections:

1. Header — one-line description + install snippet
2. Key types — terse table of all public types
3. DSL entry points — `read_batch()`, `write_batch()`, `g()`, `sub()`
4. DSL method catalog — grouped by category, one line each
5. Client API — `Client::new`, `with_api_key`, `query()`, header toggles, `dynamic_query`, `stored_query`, `body`, `send`
6. `#[register]` macro — rules + one example
7. Recipes — ~10 copy-paste patterns (find node, traverse, conditional query, add node+edge, delete, vector search, multitenancy, edge-first, repeat/union, stored query)
8. Wire protocol notes — endpoint URLs, header names, JSON shape of `DynamicQueryRequest`
9. Error variants — `SparrowError` with when each fires
10. Gotchas — bytes param panics on dynamic call; distance metadata lost after traversal step; multitenant index requires tenant on every write

## Out of Scope

- Renaming server-side `x-helix-*` header strings in `sparrow-db/`
- Porting `helix-dsl-macros` into `sparrow-macros`
- TypeScript SDK (`sparrow-ts/`) changes
- Publishing `sparrow-sdk` to crates.io
