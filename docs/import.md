# Bulk Import

`sparrow import` reads records from a JSON, CSV, or Parquet file and posts each one to a compiled HQL query on a running SparrowDB instance. It is the standard path for seeding a database, migrating data, or loading graph snapshots.

---

## Quick start

```bash
# Import nodes
sparrow import users.csv      --query CreateUser
sparrow import products.json  --query CreateProduct

# Import edges (after nodes)
sparrow import purchases.csv  --query CreatePurchase
```

---

## File formats

### JSON

The file must be a top-level array of objects. Each object's keys map directly to the named parameters of the HQL query.

```json
[
  { "name": "Alice", "age": 30, "email": "alice@example.com" },
  { "name": "Bob",   "age": 25, "email": "bob@example.com" }
]
```

### CSV

The first row must be the header. Column names become query parameter names. Cell values are type-inferred at import time:

| Cell value | Inferred type |
|-----------|---------------|
| `42`, `-7` | integer (`Number`) |
| `3.14`, `-0.5` | float (`Number`) |
| `true`, `false` (case-insensitive) | boolean (`Bool`) |
| `null`, `none` (case-insensitive) | null (`Null`) |
| empty | null (`Null`) |
| anything else | string (`String`) |

```csv
name,age,active,score
Alice,30,true,9.5
Bob,25,false,7.2
```

Leading and trailing whitespace is trimmed from both headers and values.

### Parquet

Column names become query parameter names. Types are preserved as-is from the Parquet schema and converted to their JSON equivalents. Suitable for large datasets and pipeline outputs.

```bash
sparrow import analytics.parquet --query ImportEvent
```

Format is auto-detected from the file extension (`.json`, `.csv`, `.parquet` / `.pq`). Override with `--format json|csv|parquet` when the extension is non-standard.

---

## Command reference

```
sparrow import <FILE> [OPTIONS]
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--query <NAME>` | `-q` | — | HQL query to call for every record. Required unless `--query-column` is given. |
| `--query-column <COL>` | `-c` | — | Column/field name whose value is the query for that record. Stripped before posting. `--query` is the fallback when the column is absent or empty. |
| `--target <URL>` | `-t` | `http://localhost:6969` | SparrowDB server URL. |
| `--workers <N>` | `-w` | `8` | Number of concurrent HTTP requests. |
| `--token <TOKEN>` | | — | Auth token. Also read from `SPARROW_TOKEN` env var. |
| `--dry-run` | | off | Parse the file and print a preview. No requests are sent. |
| `--format <FMT>` | `-f` | auto | Override format detection: `json`, `csv`, `parquet`. |
| `--on-error <MODE>` | | `continue` | `continue` — skip failed records and finish. `abort` — stop after first failure (in-flight requests complete). |

---

## Authentication

When auth is enabled on the instance, pass a `read_write` or `admin` token:

```bash
# Flag
sparrow import users.csv --query CreateUser --token sk-my-token

# Environment variable (preferred for scripts)
export SPARROW_TOKEN=sk-my-token
sparrow import users.csv --query CreateUser
```

The token is sent as the `x-api-key` header. See [auth.md](auth.md) for token creation.

---

## Importing a graph (nodes and edges)

SparrowDB edge queries look up both endpoint nodes before creating the edge. An edge record must therefore be imported **after** both of its endpoint nodes exist.

### Pattern A — two separate files (simplest)

```bash
# 1. Nodes first
sparrow import users.csv    --query CreateUser
sparrow import products.csv --query CreateProduct

# 2. Edges after
sparrow import purchases.csv --query CreatePurchase
```

The `CreatePurchase` query resolves both endpoints by a secondary-indexed field (e.g. `user_id`, `product_id`) before adding the edge:

```
QUERY CreatePurchase (user_id: String, product_id: String, qty: U32) =>
    user    <- N<User>({user_id: user_id})
    product <- N<Product>({product_id: product_id})
    e       <- AddE<PURCHASED>({qty: qty})::From(user)::To(product)
    RETURN e
```

### Pattern B — single file with `--query-column`

Add a routing column (e.g. `_query`) to each record. The importer reads it, strips it, and calls the named query. Sort nodes before edges in the file.

```json
[
  { "_query": "CreateUser",    "name": "Alice",   "age": 30 },
  { "_query": "CreateUser",    "name": "Bob",     "age": 25 },
  { "_query": "CreateProduct", "title": "Widget", "price": 9.99 },
  { "_query": "CreatePurchase","user_id": "u-1",  "product_id": "p-1", "qty": 2 }
]
```

```bash
sparrow import graph.json --query-column _query
```

The same column name works in CSV and Parquet. Use `--query` alongside `--query-column` as a fallback for records that omit the column.

#### Rules for `--query-column`

- The column is **removed** from the record before the HTTP call — it will not appear as a query parameter.
- If the column is absent or empty and `--query` is provided, `--query` is used.
- If the column is absent and no `--query` fallback exists, that record fails.

### Idempotent imports with Upsert

When re-running an import (e.g. after a failure), use `UpsertN` / `UpsertE` in your HQL query instead of `AddN` / `AddE` to avoid duplicate-key errors:

```
QUERY UpsertUser (user_id: String, name: String, age: U8) =>
    user <- UpsertN<User>({user_id: user_id}, {name: name, age: age})
    RETURN user
```

---

## Dry-run preview

Use `--dry-run` to inspect how the importer will map records to queries before sending any data:

```bash
sparrow import graph.json --query-column _query --dry-run
```

```
Reading graph.json (json)
  4 records parsed
(--dry-run: skipping HTTP requests)
First 3 record(s):
  → CreateUser    {"age":30,"name":"Alice"}
  → CreateUser    {"age":25,"name":"Bob"}
  → CreateProduct {"price":9.99,"title":"Widget"}
```

The output shows the resolved query name and the exact JSON body that would be posted.

---

## Performance

| Concern | Guidance |
|---------|----------|
| **Concurrency** | Default `--workers 8` is a good starting point. Increase for large files on a server with many CPU cores; decrease if the server shows back-pressure. |
| **Write throughput** | SparrowDB serialises writes through a single LMDB writer thread. Throughput is roughly `1 / avg_write_latency` regardless of worker count. Workers help by keeping the pipeline full (network latency hidden by concurrency). |
| **Large files** | Records are read entirely into memory before import begins. For very large Parquet files (> a few GB), split them beforehand. |
| **Error reporting** | With `--on-error continue` (the default), failed records are logged to the terminal and the final summary shows the failure count. Use `--on-error abort` to stop immediately and diagnose the first failure. |

---

## Export

> **Not yet implemented.**

Export functionality (`sparrow export`) is planned. In the meantime, use the HTTP API directly to stream query results:

```bash
# Export all users as newline-delimited JSON
curl -s http://localhost:6969/getAllUsers \
  -H "x-api-key: $SPARROW_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}' | jq -c '.[]'
```

For bulk graph exports, write an HQL query that returns all nodes/edges of a given type and pipe the response through `jq` or a custom script.
