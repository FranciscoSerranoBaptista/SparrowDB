# SparrowDB HTTP API

SparrowDB exposes a JSON-over-HTTP API. All endpoints accept and return `application/json`.
The server listens on `SPARROW_PORT` (default `6969`).

---

## Authentication

When the `api-key` feature is compiled in, every request must include:

```
x-api-key: <your-api-key>
```

Requests missing or with an invalid key return `403 Forbidden`.

---

## Error format

All errors return JSON with a consistent shape:

```json
{
  "error": "human-readable message",
  "code":  "ERROR_CODE"
}
```

| HTTP status | `code` value      | Meaning                                       |
|-------------|-------------------|-----------------------------------------------|
| 403         | `INVALID_API_KEY` | Missing or wrong `x-api-key` header           |
| 404         | `NOT_FOUND`       | Query name, node, edge, or label not found    |
| 500         | `GRAPH_ERROR`     | Storage or traversal error                    |
| 500         | `VECTOR_ERROR`    | HNSW / vector index error                     |

---

## Compiled query endpoint

```
POST /<QueryName>
Content-Type: application/json
```

Dispatches to a named compiled HQL operation. `<QueryName>` is the name you assigned when pushing the query with `sparrow push`.

The request body is whatever JSON the query expects. The response body is whatever the query returns.

Write operations are routed to the LMDB single-writer thread automatically — you do not need to do anything special.

**Example**

```
POST /GetUserById
{"id": "018f2e3a-1234-7abc-8def-000000000001"}
```

---

## Schema introspection

```
GET /introspect
```

Returns the schema JSON registered with the instance. No request body.

---

## HelixDB v1 compatibility

```
POST /v1/query
Content-Type: application/json
```

Accepts the HelixDB JSON DSL and translates it to SparrowDB storage operations. For full details see [`docs/V1_COMPAT_ENDPOINT.md`](V1_COMPAT_ENDPOINT.md).

**Request shape**

```json
{
  "query": {
    "queries": [
      {"Query": {"name": "<var>", "steps": [ ... ]}}
    ],
    "returns": ["<var>"]
  }
}
```

**Response shape**

```json
{
  "<var>": {
    "ids": ["<uuid-string>", ...],
    "properties": [
      {
        "id":     "<uuid-string>",
        "$id":    "<uuid-string>",
        "label":  "NodeType",
        "$label": "NodeType",
        "prop1":  "value"
      }
    ]
  }
}
```

Both `id`/`label` (SparrowDB native) and `$id`/`$label` (HelixDB aliases) are always included.

---

## Built-in operations

These are available on every instance via `POST /<name>`. No compilation step required.

### `node_details`

Fetch a single node by ID.

**Request**
```json
{"id": "<uuid-string>"}
```

**Response**
```json
{
  "node": {
    "id":    "<uuid-string>",
    "label": "NodeType",
    "title": "...",
    "<prop>": "<value>"
  },
  "found": true
}
```

`found` is `false` and `node` is `null` when the ID does not exist.

---

### `nodes_by_label`

Fetch all nodes of a given type.

**Request**
```json
{
  "label": "NodeType",
  "limit": 100
}
```

`limit` is optional.

**Response**
```json
{
  "nodes": [
    {"id": "<uuid>", "label": "NodeType", "title": "...", "<prop>": "<value>"}
  ],
  "count": 1
}
```

---

### `node_connections`

Fetch all edges and neighbouring nodes for a given node.

**Request**
```json
{"node_id": "<uuid-string>"}
```

**Response**
```json
{
  "connected_nodes": [ { "id": "...", "label": "...", ... } ],
  "incoming_edges":  [ { "id": "...", "from_node": "...", "to_node": "...", "label": "EDGE_TYPE" } ],
  "outgoing_edges":  [ { "id": "...", "from_node": "...", "to_node": "...", "label": "EDGE_TYPE" } ]
}
```

---

### `diagnostics`

Returns a snapshot of current database stats. No request body.

**Response**
```json
{
  "nodes": 1024,
  "edges": 4096,
  "vectors": {
    "total":         512,
    "active":        500,
    "soft_deleted":  12,
    "hnsw_edges":    2048,
    "entry_point_present": true
  }
}
```

---

### `hnsw_health`

BFS reachability check on the HNSW vector graph. No request body.

**Response**
```json
{
  "status":       "healthy",
  "total_active": 500,
  "reachable":    500,
  "unreachable":  0
}
```

`status` is one of `"healthy"`, `"degraded"`, or `"broken"`.

---

### `hnsw_integrity`

Checks that all HNSW edges are symmetric (each neighbour link is bidirectional). No request body.

**Response**
```json
{
  "symmetric":        true,
  "total_edges":      2048,
  "asymmetric_edges": 0
}
```

---

### `vector_soft_delete`

Marks a vector as deleted without removing it from the HNSW graph. Safe to call at any time; the vector is excluded from search results immediately.

**Request**
```json
{"id": "<uuid-string>"}
```

**Response**
```json
{"ok": true, "id": "<uuid-string>"}
```

---

### `vector_hard_delete`

Removes a vector's data record. The HNSW graph is **not** updated — call `rebuild_vector_index` afterwards to clean it up.

**Request**
```json
{"id": "<uuid-string>"}
```

**Response**
```json
{
  "ok": true,
  "id": "<uuid-string>",
  "warning": "HNSW index not updated — call rebuild_vector_index to apply"
}
```

---

### `rebuild_vector_index`

Rebuilds the HNSW graph from scratch, dropping all soft-deleted vectors in the process. This is a write operation and may take a few seconds on large graphs. No request body.

**Response**
```json
{
  "ok":            true,
  "kept":          500,
  "purged_deleted": 12
}
```

---

### `purge_soft_deleted`

Removes all soft-deleted vector records without rebuilding the HNSW graph. Cheaper than `rebuild_vector_index` when you only need to reclaim storage. No request body.

**Response**
```json
{
  "ok":       true,
  "purged":   12,
  "remaining": 500
}
```

---

## Debug/visualization endpoints

Available only when the `dev-instance` feature is compiled in.

| Method | Path                | Description                                   |
|--------|---------------------|-----------------------------------------------|
| `GET`  | `/nodes-edges`      | Top nodes + edges for graph visualization     |
| `GET`  | `/nodes-by-label`   | Node list filtered by label (query param)     |
| `GET`  | `/node-connections` | Connections for a node (query param `node_id`)|
| `GET`  | `/node-details`     | Node detail (query param `id`)                |

These are not part of the production API surface and may change without notice.
