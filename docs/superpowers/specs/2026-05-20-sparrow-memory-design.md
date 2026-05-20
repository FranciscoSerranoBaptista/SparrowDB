# sparrow-memory: Lightweight Episodic Memory for Research Agents

**Date**: 2026-05-20
**Status**: Approved
**Crate**: `sparrow-memory` (new crate in SparrowDB repo)

---

## 1. Problem

The oak9-intelligence/simorgh research agents (Eyes, orchestrators) are stateless between runs. Each run starts cold: no knowledge of what was explored before, no accumulated findings, no open questions carried forward. For agents running complex multi-hop research queries — "which interventions work for which sacred cows, under which circumstances, for which people, in which industry" — this means re-deriving everything from scratch on every run.

Graphiti solves a related problem but at full complexity (event sourcing, temporal versioning, community detection). We need something narrower: a persistent research journal for agents, backed by SparrowDB's graph+vector storage.

---

## 2. Scope

**In scope:**
- Persistent research threads that survive process restarts
- Structured findings with semantic embeddings for retrieval
- Open questions carried forward across runs
- Run summaries for context compression
- Aggregate queries over findings (distinct entity counts, deduplication)
- Schema-free operation (no HQL compiler dependency)

**Out of scope:**
- Turn-by-turn conversation logging
- Multi-user access control
- Replication or distributed storage
- Any knowledge of the domain graph's node types

---

## 3. Architecture

`sparrow-memory` is a new Rust crate inside the SparrowDB repo. It depends on `sparrow-db` with `features = ["lmdb", "vectors"]`. No HTTP server. No HQL compiler. It opens a dedicated LMDB environment at a caller-supplied path, fully separate from the domain database.

```
┌─────────────────────────────────────────┐
│  Research Agent (Rust)                  │
│                                         │
│  let store = MemoryStore::open(cfg)?;   │
│  let thread = store.thread("Eye1",      │
│      "sacred-cow-interventions")?;      │
│  let run = thread.start_run()?;         │
│  run.record_finding(Finding { .. })?;   │
│  let ctx = thread.recall("polarity")?;  │
└────────────────┬────────────────────────┘
                 │ direct crate dep, no HTTP
                 ▼
┌─────────────────────────────────────────┐
│  sparrow-memory                         │
│  schema-free graph + vector operations  │
└────────────────┬────────────────────────┘
                 │ sparrow-db (lmdb + vectors)
                 ▼
┌─────────────────────────────────────────┐
│  SparrowGraphStorage (LMDB)             │
│  /var/data/agent-memory/                │  ← separate from simorgh/helix-db
└─────────────────────────────────────────┘
```

The domain graph (simorgh/helix-db) and the memory graph are physically separate environments. The only coupling is opaque `u128` entity IDs that `Finding` nodes carry as foreign references — the memory layer never dereferences them.

---

## 4. Graph Schema

Node labels and edge names are string constants defined in library code. No schema compilation. Properties are `Value` maps stored directly via `SparrowGraphStorage`.

### Nodes

| Label | Key Properties | Notes |
|---|---|---|
| `agent` | `name: String`, `agent_type: String`, `created_at: Date` | One per agent identity. Upserted by name. |
| `research_thread` | `name: String`, `goal: String`, `agent_name: String`, `status: String`, `created_at: Date`, `updated_at: Date` | A named inquiry spanning multiple runs. Upserted by `(agent_name, name)`. |
| `agent_run` | `thread_id: U128`, `agent_name: String`, `started_at: Date`, `ended_at: Date`, `status: String` | One per execution. status: `running` / `completed` / `interrupted`. |
| `finding` | `claim: String`, `confidence: F32`, `entity_id: U128?`, `entity_label: String?`, `run_id: U128`, `thread_id: U128`, `created_at: Date` + `metadata: Object` | Core memory unit. `claim` is embedded for semantic recall. |
| `open_question` | `question: String`, `priority: String`, `thread_id: U128`, `run_id: U128`, `status: String`, `created_at: Date` | Unresolved inquiry. status: `open` / `resolved`. |
| `run_summary` | `summary: String`, `finding_count: U32`, `question_count: U32`, `run_id: U128`, `thread_id: U128`, `created_at: Date` | Compacted memory of a completed run. `summary` is embedded. Prevents unbounded context growth. |

### Edges

```
agent           → OWNS          → research_thread   (agent identity owns its threads)
research_thread → HAS_RUN       → agent_run         (each run belongs to a thread)
agent_run       → FOLLOWS       → agent_run         (temporal chain within a thread)
agent_run       → PRODUCED      → finding           (findings written in a run)
agent_run       → RAISED        → open_question     (questions opened in a run)
agent_run       → CARRIED       → open_question     (open questions forwarded from prior run)
finding         → ANSWERS       → open_question     (when a finding resolves a question)
agent_run       → SUMMARIZED_AS → run_summary       (one summary per completed run)
```

### Secondary indices

- `finding.thread_id` — fast scan of all findings for a thread
- `finding.entity_id` — fast scan of all findings referencing a domain entity (used by `count_distinct`)
- `open_question.thread_id` — fast scan of all questions for a thread
- `open_question.status` — fast scan filtered to `open` status
- `agent_run.thread_id` — fast scan of all runs for a thread

---

## 5. The `Finding` Type

The `Finding` is the core memory unit. Agents must provide `claim` and `confidence`. Everything else is optional.

```rust
pub struct Finding {
    /// The finding itself — what the agent concluded. This is what gets embedded.
    pub claim: String,

    /// 0.0–1.0. Agent's confidence in the claim.
    pub confidence: f32,

    /// Opaque foreign reference into the domain graph (e.g., a sacred_cow node ID).
    /// The memory layer never dereferences this — only used for aggregate queries.
    pub entity_id: Option<u128>,

    /// Label of the referenced entity, e.g. "sacred_cow", "intervention", "coachee".
    /// Used as a filter dimension in aggregate queries.
    pub entity_label: Option<String>,

    /// Agent-specific structured metadata. Arbitrary — the library does not inspect it.
    /// Example: {"client_id": "uuid", "industry": "tech", "session_count": 4}
    /// Uses sparrow-db's Value::Object type, not serde_json.
    pub metadata: HashMap<String, Value>,
}
```

The `claim` string is embedded at write time using SparrowDB's configured embedding model. If no embedding model is configured, semantic recall falls back to BM25 only.

---

## 6. API Surface

```rust
// ── Initialisation ──────────────────────────────────────────────────

pub struct MemoryStore { .. }

impl MemoryStore {
    pub fn open(config: MemoryConfig) -> Result<Self>;
}

pub struct MemoryConfig {
    pub path: String,
    pub embedding_model: Option<String>,  // None = BM25-only recall
    pub db_max_size_gb: Option<usize>,
}

// ── Thread handle ────────────────────────────────────────────────────

impl MemoryStore {
    /// Get an existing thread or create one. Idempotent.
    pub fn thread(&self, agent: &str, name: &str, goal: &str) -> Result<ThreadHandle>;

    /// List all threads for an agent.
    pub fn threads(&self, agent: &str) -> Result<Vec<ThreadSummary>>;
}

pub struct ThreadHandle { .. }

impl ThreadHandle {
    /// Start a new run against this thread. Chains FOLLOWS from the previous run.
    pub fn start_run(&self) -> Result<RunHandle>;

    /// Retrieve context for the next Claude call.
    /// Returns: recent run summaries + semantically similar findings + all open questions.
    pub fn recall(&self, query: &str) -> Result<RecallResult>;

    /// Count distinct entity_ids of a given label across all findings in this thread.
    /// Solves the double-counting problem: a client with 3 sacred cow findings counts once.
    pub fn count_distinct(&self, entity_label: &str) -> Result<usize>;

    /// All findings referencing a specific domain entity.
    pub fn findings_for_entity(&self, entity_id: u128) -> Result<Vec<StoredFinding>>;
}

// ── Run handle ───────────────────────────────────────────────────────

pub struct RunHandle { .. }

impl RunHandle {
    pub fn record_finding(&self, finding: Finding) -> Result<FindingId>;
    pub fn raise_question(&self, question: &str, priority: Priority) -> Result<QuestionId>;
    pub fn answer_question(&self, question_id: QuestionId, finding_id: FindingId) -> Result<()>;

    /// Mark run complete, write a summary node (embedded), carry open questions forward.
    pub fn complete(self, summary: &str) -> Result<()>;

    /// Mark run interrupted — open questions are still carried forward on next start_run.
    pub fn interrupt(self) -> Result<()>;
}

pub enum Priority { High, Medium, Low }

// ── Recall result ────────────────────────────────────────────────────

pub struct RecallResult {
    /// Summaries of the last N completed runs, newest first.
    pub recent_summaries: Vec<StoredSummary>,

    /// Top-K findings semantically similar to the query, across all runs in the thread.
    pub relevant_findings: Vec<StoredFinding>,

    /// All currently open questions for this thread.
    pub open_questions: Vec<StoredQuestion>,
}
```

---

## 7. Retrieval Strategy

`thread.recall(query)` performs three operations and merges results:

1. **Recent summaries** — walk `HAS_RUN` edges in reverse temporal order, fetch the last 3 `run_summary` nodes. Always included regardless of query.

2. **Semantic finding search** — vector search over embedded `finding.claim` fields scoped to this thread. Falls back to BM25 if no embedding model is configured. Returns top 20 by default.

3. **Open questions** — secondary index scan on `(thread_id, status=open)`. All open questions are always returned; they are small and the agent must see all of them.

The `RecallResult` is designed to be serialised directly into a Claude system prompt prefix. Agents format it themselves — the library does not generate prompt text.

---

## 8. Aggregate Queries

The research questions driving this library ("count clients with sacred cow X, don't double-count") require aggregate operations over structured findings, not text search.

`count_distinct(entity_label)` scans the `finding.entity_id` secondary index filtered by `entity_label`, deduplicates on `entity_id`, and returns the count. A client carrying 3 sacred cows appears in 3 findings but `count_distinct("coachee")` returns the correct unique client count. Findings where `entity_id` is `None` are excluded from this count — they are not entity-anchored observations.

More complex aggregates (cross-tabulation by industry, correlation between metadata fields) are expected to be done by the agent itself: it calls `findings_for_entity` or iterates findings and processes `metadata` fields. The library intentionally does not build a query DSL — the agent's Rust code is the query layer.

---

## 9. Embedding Configuration

Embeddings are optional. Without them, recall uses BM25 only — sufficient for exact-term retrieval but weaker for semantic similarity. With an embedding model configured, `finding.claim` and `run_summary.summary` are embedded at write time using SparrowDB's existing embedding infrastructure.

The library configures its own SparrowDB instance with the embedding model. Agents do not manage embeddings directly.

---

## 10. Crate Layout

```
sparrow-memory/
  Cargo.toml           — depends on sparrow-db (lmdb, vectors), no compiler feature
  src/
    lib.rs             — public API re-exports
    store.rs           — MemoryStore, MemoryConfig, open/init
    thread.rs          — ThreadHandle, recall, count_distinct, findings_for_entity
    run.rs             — RunHandle, record_finding, raise_question, complete, interrupt
    graph.rs           — node/edge write helpers over SparrowGraphStorage
    recall.rs          — RecallResult assembly, vector + BM25 hybrid
    types.rs           — Finding, StoredFinding, RecallResult, Priority, ids
    indices.rs         — secondary index definitions
    error.rs           — MemoryError wrapping GraphError
```

---

## 11. Usage Example

```rust
let store = MemoryStore::open(MemoryConfig {
    path: "/var/data/agent-memory".to_string(),
    embedding_model: Some("text-embedding-3-small".to_string()),
    db_max_size_gb: Some(4),
})?;

let thread = store.thread(
    "Eye1",
    "sacred-cow-interventions",
    "Which interventions work for which sacred cows, when, for whom, in which industry?",
)?;

// At run start — build context for Claude
let ctx = thread.recall("polarity interventions finance sector")?;
// ctx.recent_summaries  → last 3 run summaries
// ctx.relevant_findings → top-20 semantically similar past findings
// ctx.open_questions    → ["Does industry moderate intervention effectiveness?", ...]

// During run — record what was found
let run = thread.start_run()?;

let fid = run.record_finding(Finding {
    claim: "Confrontational reframing backfires in hierarchical cultures \
            when the sacred cow is authority-based".to_string(),
    confidence: 0.82,
    entity_id: Some(sacred_cow_node_id),
    entity_label: Some("sacred_cow".to_string()),
    metadata: HashMap::from([
        ("industry".to_string(), Value::String("finance".to_string())),
        ("coachee_count".to_string(), Value::U32(7)),
    ]),
})?;

run.answer_question(open_q_id, fid)?;

run.complete(
    "Identified authority-culture moderation effect on confrontational reframing. \
     Finance sector shows strongest suppression effect (n=7 coachees)."
)?;

// Aggregate — correctly deduplicates clients who hold multiple sacred cows
let unique_clients = thread.count_distinct("coachee")?;
```
