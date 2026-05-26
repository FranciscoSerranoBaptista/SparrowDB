# docs

Project documentation for SparrowDB. Contains API references, design plans, compiler notes, and bug write-ups.

## Files

| File | Description |
|---|---|
| `HTTP_API.md` | HTTP API reference: endpoints, request/response shapes, auth, token management |
| `HQL.md` | HQL language guide: syntax, compilation, types, query patterns |
| `auth.md` | Auth operator guide: bootstrapping, roles, token lifecycle, `SPARROW_API_KEY` |
| `import.md` | Bulk import guide: `sparrow import`, supported formats (JSON/CSV/Parquet), graph import patterns |
| `llms.txt` | LLM-friendly summary of the full project: concepts, API surface, crate layout |
| `RUNTIME_HQL_INTERPRETER_PLAN.md` | Design plan for the `/__hql_runtime_eval` dynamic HQL interpreter |
| `TS_CLIENT_GENERATION_PLAN.md` | Plan for TypeScript client/SDK code generation |

## Directories

| Directory | Description |
|---|---|
| `bugs/` | Bug reports and investigation notes — one Markdown file per bug |
| `superpowers/plans/` | Feature implementation plans (dated Markdown files) |
| `superpowers/specs/` | Design specs for major subsystems |

## Legacy / Migration

| Document | Status |
|----------|--------|
| [`V1_COMPAT_ENDPOINT.md`](V1_COMPAT_ENDPOINT.md) | Deprecated — HelixDB migration bridge. Deleted after simorgh migrates to native HQL. |
