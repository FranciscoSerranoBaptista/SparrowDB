# sparrow-memory

Lightweight episodic memory layer for AI research agents, backed by SparrowDB. Provides persistent, queryable memory across agent runs: threads, findings, questions, and vector-indexed recall — without requiring a running server process.

## Build

```bash
cargo build -p sparrow-memory
```

## Test

```bash
cargo test -p sparrow-memory
```

## Usage

```rust
use sparrow_memory::{MemoryConfig, MemoryStore, Finding, Priority};

let store = MemoryStore::open(MemoryConfig {
    path: "/tmp/agent-memory".to_string(),
    db_max_size_gb: Some(1),
    embedding_model: None,
})?;

let thread = store.create_thread("research session")?;
let finding = Finding {
    claim: "Users prefer shorter onboarding flows".to_string(),
    confidence: 0.85,
    entity_id: None,
    entity_label: None,
    metadata: Default::default(),
};
store.add_finding(thread, finding)?;
```

## Key types

| Type | Description |
|---|---|
| `MemoryStore` | Top-level handle; open with `MemoryStore::open(config)` |
| `MemoryConfig` | Path, optional max DB size, optional embedding model |
| `ThreadHandle` | Represents a single agent run / conversation |
| `RunHandle` | Represents a sub-run within a thread |
| `Finding` | A single concluded fact with confidence, optional entity reference, and metadata |
| `Priority` | Priority tag for a finding |
| `RecallResult` | Returned by vector recall queries |

## Dependencies

Depends on `sparrow-core` with `lmdb` and `vectors` features (no `server` feature — runs embedded, not as an HTTP service).
