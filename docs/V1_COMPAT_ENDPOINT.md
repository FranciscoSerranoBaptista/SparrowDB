# HelixDB v1/query Compatibility Endpoint

## What This Is

`POST /v1/query` is a compatibility shim that translates the HelixDB JSON DSL into
SparrowDB storage operations, allowing existing HelixDB clients to migrate to SparrowDB
without rewriting their query layer first.

The implementation lives in
`sparrow-db/src/sparrow_gateway/v1_compat/mod.rs`.

---

## Why It Exists

Simorgh (the API layer driving this graph) currently builds all queries as HelixDB JSON DSL
and posts them to `POST /v1/query`. SparrowDB has no such endpoint — queries are compiled
HQL named operations or dynamic HQL via `/__hql_runtime_eval`.

Rather than rewrite the entire simorgh query layer before migrating data, we added a bridge:
simorgh talks to SparrowDB at `/v1/query` as if it were HelixDB. After data migration
stabilises, the query layer is rewritten to use HQL properly.

This is the recommended migration path from `docs/architecture/sparrowdb-migration.md`
(Phase 2a, `/__hql_runtime_eval` approach, but via static translation rather than HQL text).

---

## Activation

The endpoint is registered unconditionally (compiled into the binary). It is available at
`POST /v1/query` on any running SparrowDB instance.

The axum route is added before `/{*path}` in `gateway.rs`. Without that specific route,
the `/{*path}` wildcard handler rejects paths containing `/`, so `/v1/query` would return
400.

Write operations (`"request_type": "write"`) are routed to the dedicated LMDB writer thread
(`__v1_compat_write` handler). Read operations go to the reader thread pool
(`__v1_compat_read` handler).

---

## Supported HelixDB DSL Operations

| Step | Translation |
|---|---|
| `{"NWhere": {"Eq": ["$label", {"String": T}]}}` | `NFromType(T)` |
| `{"NWhere": {"And": [label_eq, ...rest]}}` | `NFromType` from label eq + `FilterItems` for rest |
| `{"NWhere": {"And": [...props]}}` (no label) | `FilterItems` scan only |
| `{"EWhere": {"Eq": ["$label", {"String": T}]}}` | `EFromType(T)` — returns all edges with that label |
| `{"EWhere": {"And": [label_eq, ...rest]}}` | `EFromType` from label eq + `FilterItems` for rest |
| `{"N": {"Ids": ["uuid-str", ...]}}` | Direct `storage.get_node` by UUID |
| `{"Out": "EDGE"}` | `OutStep { edge_label: EDGE }` |
| `{"In": "EDGE"}` | `InStep { edge_label: EDGE }` |
| `{"OutN": "EDGE"}` | `OutStep { edge_label: EDGE }` — identical to `Out` |
| `{"InN": "EDGE"}` | `InStep { edge_label: EDGE }` — identical to `In` |
| `{"Where": condition}` | `FilterItems` with translated condition |
| `{"AddN": {label, properties}}` | `MutationOp::AddNode` |
| `{"Inject": "var"}` | Sets seed_var for next traversal step |
| `{"AddE": {label, to: {Var: T}, properties}}` | `MutationOp::AddEdge { from_var, to_var }` |
| `{"SetProperty": ["key", val]}` | `MutationOp::UpdateNodes` |
| `{"Drop": null}` | `MutationOp::DropNodes` |
| `{"VectorSearchNodes": {...}}` | `NFromType(label)` + `SearchVec(vector, k)` |
| `{"Id": null}` | No-op (IDs always returned) |
| `{"ValueMap": [fields]}` | No-op (all fields always returned) |
| `{"Project": [[from,to],...]}` | No-op (all fields always returned) |

**Not supported:** `Repeat` (recursive traversal), `SearchVecText`, `SearchKeyword`, `OutN`/`InN` with `null` label (must supply a string edge label).

**Notes on `EWhere`:** Returns edges matching the predicate, analogous to `NWhere` for nodes.
Edges in the response carry `id`, `label`, `from_node`, and `to_node` fields (plus `$id`/`$label` compat aliases).

**Notes on `OutN`/`InN`:** These are now functional and translate identically to `Out`/`In`.
The `N` suffix has no effect in the v1_compat layer — both forms follow the edge and return destination/source nodes.
Prefer `Out`/`In` in new code for clarity; `OutN`/`InN` are supported for HelixDB migration compat.

---

## ID Format Change

HelixDB uses sequential `i64` IDs. SparrowDB uses `u128` UUIDs serialised as strings.

**Response format** (both HelixDB and v1_compat):
```json
{
  "result_name": {
    "ids": ["<uuid-string>", ...],
    "properties": [
      {
        "id":     "<uuid-string>",
        "$id":    "<uuid-string>",
        "label":  "node_type",
        "$label": "node_type",
        "prop1":  "value1"
      }
    ]
  }
}
```

Both `id`/`label` (SparrowDB native) and `$id`/`$label` (HelixDB compat aliases) are
included so simorgh can migrate field access gradually.

**Simorgh changes required:**
- `extract_ids`: change `v.as_i64()` to `v.as_str().map(String::from)` (or equivalent)
- `ensure_edge(from_id: i64, to_id: i64)` → `(from_id: String, to_id: String)`
- `create_edge_query(&[i64], &[i64])` → `(&[String], &[String])`
- `set_properties_query(&[i64], ...)` → `(&[String], ...)`

---

## Property Value Format

HelixDB uses wrapped property values in `AddN`/`AddE`/`SetProperty`:
```json
["prop_name", {"Value": {"String": "foo"}}]
["prop_name", {"Value": {"I64": 42}}]
["prop_name", {"Value": {"F64": 1.5}}]
["prop_name", {"Value": {"Bool": true}}]
["prop_name", {"Value": {"F64Array": [1.0, 2.0]}}]
```

v1_compat strips the `{"Value": ...}` wrapper before writing to SparrowDB storage.

For `NWhere`/`Where` condition values (not property writes), the format has no outer wrapper:
```json
{"Eq": ["prop", {"String": "foo"}]}
{"Eq": ["prop", {"I64": 42}]}
```

---

## Multi-Query Execution

HelixDB supports multiple named queries in one request (used by `create_edge_query`):
```json
{
  "queries": [
    {"Query": {"name": "src", "steps": [{"N": {"Ids": ["uuid-a"]}}]}},
    {"Query": {"name": "tgt", "steps": [{"N": {"Ids": ["uuid-b"]}}]}},
    {"Query": {"name": "edge", "steps": [
      {"Inject": "src"},
      {"AddE": {"label": "KNOWS", "to": {"Var": "tgt"}, "properties": []}}
    ]}}
  ],
  "returns": ["edge"]
}
```

v1_compat processes each named query in order, threading results through a shared live
store. `Inject: "varname"` seeds the next traversal from a previous query's result.
`AddE { to: {Var: "varname"} }` uses the named variable as the edge target.

---

## What to Do After Migration

Once simorgh is stable on SparrowDB and the data migration is complete:

1. Rewrite simorgh's query emitter to use HQL text via `/__hql_runtime_eval`, or
   compile named HQL operations via `sparrow push` and call them as `POST /<QueryName>`.
2. Remove the v1_compat module and the `/v1/query` axum route.
3. The `$id`/`$label` aliases in node responses can also be removed at that point.

The v1_compat endpoint is a migration aid, not a permanent API. It should not be
present in a fully migrated production deployment.

---

## Known Limitations

- **N:Ids with i64**: If simorgh passes integer IDs (HelixDB-era), they are treated as
  u128 values (likely wrong). Only UUID strings in N:Ids work correctly.
- **Repeat (recursive traversal)**: Not implemented. Returns an error.
- **Count step**: The count is returned as the length of the `ids` array in the response.
  Simorgh reads count from `resp["exists"]["count"]` — this path is not currently populated.
  `ensure_edge` will need to read `ids.len() > 0` instead.
- **SearchVecText**: Not supported (requires embedding model in the request path).
- **Concurrent writes**: All writes route through the LMDB single-writer thread, same as
  normal compiled queries. No additional locking needed.
