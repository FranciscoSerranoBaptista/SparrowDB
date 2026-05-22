# docs

Project documentation for SparrowDB. Contains API references, design plans, compiler notes, and bug write-ups.

## Files

| File | Description |
|---|---|
| `HTTP_API.md` | HTTP API reference: endpoints, request/response shapes, auth, token management |
| `auth.md` | Auth operator guide: bootstrapping, roles, token lifecycle, `SPARROW_API_KEY` |
| `import.md` | Bulk import guide: `sparrow import`, supported formats (JSON/CSV/Parquet), graph import patterns |
| `llms.txt` | LLM-friendly summary of the full project: concepts, API surface, crate layout |
| `RUNTIME_HQL_INTERPRETER_PLAN.md` | Design plan for the `/__hql_runtime_eval` dynamic HQL interpreter |
| `TS_CLIENT_GENERATION_PLAN.md` | Plan for TypeScript client/SDK code generation |
| `V1_COMPAT_ENDPOINT.md` | Notes on the v1-compatibility endpoint |

## Directories

| Directory | Description |
|---|---|
| `bugs/` | Bug reports and investigation notes — one Markdown file per bug |
| `superpowers/plans/` | Feature implementation plans (dated Markdown files) |
| `superpowers/specs/` | Design specs for major subsystems |
