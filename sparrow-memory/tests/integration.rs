use sparrow_memory::graph::{ids_from_index, out_neighbors, write_edge, write_node, write_node_indexed};
use sparrow_memory::{MemoryConfig, MemoryStore};
use sparrow_db::{
    protocol::value::Value,
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, storage_methods::StorageMethods, version_info::VersionInfo},
        traversal_core::config::{Config, GraphConfig},
        types::SecondaryIndex,
    },
};
use tempfile::TempDir;

fn open_test_storage() -> (SparrowGraphStorage, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut config = Config::default();
    config.db_max_size_gb = Some(1);
    let vi = VersionInfo::default();
    let storage = SparrowGraphStorage::new(dir.path().to_str().unwrap(), config, vi).unwrap();
    (storage, dir)
}

fn open_test_storage_with_index(idx_name: &str) -> (SparrowGraphStorage, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut config = Config::default();
    config.db_max_size_gb = Some(1);
    config.graph_config = Some(GraphConfig {
        secondary_indices: Some(vec![SecondaryIndex::Index(idx_name.to_string())]),
    });
    let vi = VersionInfo::default();
    let storage = SparrowGraphStorage::new(dir.path().to_str().unwrap(), config, vi).unwrap();
    (storage, dir)
}

#[test]
fn test_write_and_read_node() {
    let (storage, _dir) = open_test_storage();
    let props = vec![
        ("claim", Value::String("test finding".to_string())),
        ("confidence", Value::F32(0.9)),
    ];
    let id = write_node(&storage, "finding", props).unwrap();
    let arena = bumpalo::Bump::new();
    let rtxn = storage.graph_env.read_txn().unwrap();
    let node = storage.get_node(&rtxn, id, &arena).unwrap();
    assert_eq!(node.label, "finding");
    assert_eq!(
        node.get_property("claim"),
        Some(&Value::String("test finding".to_string()))
    );
}

#[test]
fn test_write_edge_and_neighbors() {
    let (storage, _dir) = open_test_storage();
    let from_id = write_node(&storage, "person", vec![]).unwrap();
    let to_id = write_node(&storage, "person", vec![]).unwrap();
    write_edge(&storage, from_id, to_id, "rel").unwrap();
    let neighbors = out_neighbors(&storage, from_id, "rel").unwrap();
    assert!(neighbors.contains(&to_id));
}

#[test]
fn test_indexed_node() {
    let (storage, _dir) = open_test_storage_with_index("test:idx");
    let id_a = write_node_indexed(&storage, "thing", vec![("name", Value::String("a".to_string()))], "test:idx", Value::U128(42)).unwrap();
    let id_b = write_node_indexed(&storage, "thing", vec![("name", Value::String("b".to_string()))], "test:idx", Value::U128(42)).unwrap();
    let ids = ids_from_index(&storage, "test:idx", &Value::U128(42)).unwrap();
    assert!(ids.contains(&id_a), "first node should be in index");
    assert!(ids.contains(&id_b), "second node should be in index");
    assert_eq!(ids.len(), 2, "both nodes under same key, no clobbering");
}

// ── MemoryStore / ThreadHandle / RunHandle / Recall integration tests ─────────

fn open_memory_store() -> (MemoryStore, TempDir) {
    let dir = TempDir::new().unwrap();
    let store = MemoryStore::open(MemoryConfig {
        path: dir.path().to_str().unwrap().to_string(),
        db_max_size_gb: Some(1),
        embedding_model: None,
    })
    .unwrap();
    (store, dir)
}

#[test]
fn test_memory_store_opens() {
    let (store, _dir) = open_memory_store();
    assert_eq!(store.index_names().len(), 6, "expected 6 secondary indices");
}

#[test]
fn test_thread_idempotent() {
    let (store, _dir) = open_memory_store();
    let t1 = store.thread("agent-a", "thread-x", "goal").unwrap();
    let t2 = store.thread("agent-a", "thread-x", "goal").unwrap();
    assert_eq!(t1.thread_id(), t2.thread_id(), "get_or_create should return same thread");
}

#[test]
fn test_run_record_finding_and_complete() {
    let (store, _dir) = open_memory_store();
    let thread = store.thread("agent-a", "run-test", "understand things").unwrap();

    let run = thread.start_run().unwrap();
    run.record_finding(sparrow_memory::types::Finding {
        claim: "The sky is blue".to_string(),
        confidence: 0.95,
        entity_id: None,
        entity_label: None,
        metadata: Default::default(),
    })
    .unwrap();
    run.complete("First run done").unwrap();

    let recall = thread.recall("").unwrap();
    assert_eq!(recall.recent_summaries.len(), 1, "expected 1 summary");
    assert_eq!(recall.relevant_findings.len(), 1, "expected 1 finding");
    assert_eq!(recall.open_questions.len(), 0, "expected 0 open questions");
}

#[test]
fn test_recall_accumulates_across_runs() {
    let (store, _dir) = open_memory_store();
    let thread = store.thread("agent-b", "multi-run", "deep dive").unwrap();

    // Run 1: 2 findings, 1 open question
    {
        let run = thread.start_run().unwrap();
        run.record_finding(sparrow_memory::types::Finding {
            claim: "Finding A".to_string(),
            confidence: 0.8,
            entity_id: None,
            entity_label: None,
            metadata: Default::default(),
        })
        .unwrap();
        run.record_finding(sparrow_memory::types::Finding {
            claim: "Finding B".to_string(),
            confidence: 0.7,
            entity_id: None,
            entity_label: None,
            metadata: Default::default(),
        })
        .unwrap();
        run.raise_question("What is X?", sparrow_memory::types::Priority::High)
            .unwrap();
        run.complete("Run 1 done").unwrap();
    }

    // Run 2: 1 finding, open question carried forward
    {
        let run = thread.start_run().unwrap();
        run.record_finding(sparrow_memory::types::Finding {
            claim: "Finding C".to_string(),
            confidence: 0.9,
            entity_id: None,
            entity_label: None,
            metadata: Default::default(),
        })
        .unwrap();
        run.complete("Run 2 done").unwrap();
    }

    let recall = thread.recall("").unwrap();
    assert_eq!(recall.recent_summaries.len(), 2, "expected 2 summaries");
    assert_eq!(recall.relevant_findings.len(), 3, "expected 3 findings total");
    assert_eq!(recall.open_questions.len(), 1, "expected 1 open question still open");
}

#[test]
fn test_count_distinct_deduplicates() {
    let (store, _dir) = open_memory_store();
    let thread = store
        .thread("agent-c", "distinct-test", "count entities")
        .unwrap();

    let run = thread.start_run().unwrap();
    // 4 findings: 2 for entity 101, 2 for entity 202 — all same label
    for _ in 0..2 {
        run.record_finding(sparrow_memory::types::Finding {
            claim: "claim about alpha".to_string(),
            confidence: 0.5,
            entity_id: Some(101),
            entity_label: Some("sacred_cow".to_string()),
            metadata: Default::default(),
        })
        .unwrap();
    }
    for _ in 0..2 {
        run.record_finding(sparrow_memory::types::Finding {
            claim: "claim about beta".to_string(),
            confidence: 0.5,
            entity_id: Some(202),
            entity_label: Some("sacred_cow".to_string()),
            metadata: Default::default(),
        })
        .unwrap();
    }
    run.complete("distinct run done").unwrap();

    let count = thread.count_distinct("sacred_cow").unwrap();
    assert_eq!(count, 2, "expected 2 distinct entity IDs");
}

#[test]
fn test_findings_for_entity() {
    let (store, _dir) = open_memory_store();
    let thread = store
        .thread("agent-d", "entity-filter", "filter by entity")
        .unwrap();

    let run = thread.start_run().unwrap();
    // Finding for entity 10
    run.record_finding(sparrow_memory::types::Finding {
        claim: "about entity 10".to_string(),
        confidence: 0.9,
        entity_id: Some(10),
        entity_label: Some("thing".to_string()),
        metadata: Default::default(),
    })
    .unwrap();
    // Finding for entity 20
    run.record_finding(sparrow_memory::types::Finding {
        claim: "about entity 20".to_string(),
        confidence: 0.8,
        entity_id: Some(20),
        entity_label: Some("thing".to_string()),
        metadata: Default::default(),
    })
    .unwrap();
    run.complete("entity-filter done").unwrap();

    let findings = thread.findings_for_entity(10).unwrap();
    assert_eq!(findings.len(), 1, "expected exactly 1 finding for entity 10");
    assert_eq!(findings[0].entity_id, Some(10));
}
