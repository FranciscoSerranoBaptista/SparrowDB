 ---
  What you already have (strong foundation)
  - Graph storage + traversal
  - Vector index (BM25 + reranker + embeddings)
  - HQL compiler + runtime eval
  - MCP server (AI agent integration)
  - REST gateway with built-in queries
  - Docker-based local instances
  - CLI with full lifecycle (init/build/start/stop/push/backup/migrate)
  - TypeScript + Rust SDKs
  
  ---
  What's missing for a well-rounded product
  
  Tier 1 — High value, users expect it
  
  ┌────────────────────────┬────────────────────────────────────────────────────────────────────────────────────────────────┐
  │          Gap           │                                         Why it matters                                         │
  ├────────────────────────┼────────────────────────────────────────────────────────────────────────────────────────────────┤
  │                        │ Every mature DB has one — Neo4j Browser, Mongo Compass, pgAdmin. Users need to explore the     │
  │ Visual UI / Studio     │ graph, run ad-hoc HQL, inspect schema, visualise query results as a graph. This is the single  │
  │                        │ highest-leverage thing you mentioned.                                                          │
  ├────────────────────────┼────────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Schema migrations      │ Right now migrations are manual. A versioned migration system (like Flyway/Liquibase or Prisma │
  │ (beyond migrate CLI)   │  Migrate) lets teams evolve schema safely without manual intervention.                         │
  ├────────────────────────┼────────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Query explain /        │ Users need to see why a query is slow — index hits, traversal depth, vector scan cost. Without │
  │ profiling              │  this, tuning is guesswork.                                                                    │
  ├────────────────────────┼────────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Authentication &       │ The key_verification.rs suggests API key auth exists, but no user/role/permission model. Teams │
  │ multi-tenancy          │  need row-level or collection-level access control.                                            │
  └────────────────────────┴────────────────────────────────────────────────────────────────────────────────────────────────┘
  
  Tier 2 — Expected once you grow
  
  ┌─────────────────────────┬───────────────────────────────────────────────────────────────────────────────────────────────┐
  │           Gap           │                                        Why it matters                                         │
  ├─────────────────────────┼───────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Replication /           │ Single-node is fine for dev but customers will want HA. Even a read-replica model unblocks a  │
  │ clustering              │ lot.                                                                                          │
  ├─────────────────────────┼───────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Streaming / change data │ Let downstream systems subscribe to graph mutations. Critical for event-driven architectures. │
  │  capture                │                                                                                               │
  ├─────────────────────────┼───────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Import/export tools     │ Bulk load from JSON/CSV/Parquet. Right now there's no sparrow import command.                 │
  ├─────────────────────────┼───────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Backup to cloud storage │ backup exists but presumably writes local. S3/GCS target is table stakes for production.      │
  ├─────────────────────────┼───────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Metrics dashboard       │ sparrow-metrics exists but there's no built-in observability surface — Prometheus endpoint,   │
  │                         │ or at minimum sparrow status with live query rates/latency.                                   │
  └─────────────────────────┴───────────────────────────────────────────────────────────────────────────────────────────────┘
  
  Tier 3 — Differentiators for the AI-native angle
  
  ┌───────────────────────┬─────────────────────────────────────────────────────────────────────────────────────────────────┐
  │          Gap          │                                         Why it matters                                          │
  ├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Hybrid search tuning  │ Your BM25 + vector + graph traversal combo is genuinely novel. A UI that lets users weight      │
  │ UI                    │ these and see result explanations would be a strong differentiator.                             │
  ├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Embedding provider    │ embedding_providers is in the gateway — users need a way to configure, test, and switch         │
  │ management UI         │ providers without editing config files.                                                         │
  ├───────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────────────┤
  │ Agent memory browser  │ sparrow-memory is interesting — a UI to inspect/replay/prune an agent's episodic memory would   │
  │                       │ be unique in the market.                                                                        │
  └───────────────────────┴─────────────────────────────────────────────────────────────────────────────────────────────────┘
