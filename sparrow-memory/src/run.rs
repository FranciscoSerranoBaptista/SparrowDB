use std::sync::Arc;
use sparrow_db::{
    protocol::value::Value,
    sparrow_engine::storage_core::SparrowGraphStorage,
    utils::{items::Node, properties::ImmutablePropertiesMap},
};

use crate::{
    error::MemoryError,
    graph::{add_to_index, ids_from_index, read_node_props, write_edge, write_node_indexed},
    indices::{FINDING_ENTITY_ID, FINDING_THREAD_ID, QUESTION_THREAD_ID, RUN_THREAD_ID, SUMMARY_THREAD_ID},
    types::{Finding, FindingId, Priority, QuestionId},
};

pub struct RunHandle {
    pub(crate) storage: Arc<SparrowGraphStorage>,
    pub(crate) thread_id: u128,
    pub(crate) run_id: u128,
}

impl RunHandle {
    pub fn run_id(&self) -> u128 {
        self.run_id
    }

    /// Create a new run node and attach it to the thread.
    pub fn create(storage: Arc<SparrowGraphStorage>, thread_id: u128) -> Result<Self, MemoryError> {
        // Write the new run node indexed under this thread
        let run_id = write_node_indexed(
            &storage,
            "agent_run",
            vec![
                ("thread_id", Value::U128(thread_id)),
                ("status",    Value::String("running".to_string())),
            ],
            RUN_THREAD_ID,
            Value::U128(thread_id),
        )?;

        // Wire up FOLLOWS edge to the previous run (most recent by ID)
        let prev_runs = ids_from_index(&storage, RUN_THREAD_ID, &Value::U128(thread_id))?;
        let prev = prev_runs.iter().copied().filter(|&id| id != run_id).max();
        if let Some(prev_id) = prev {
            write_edge(&storage, run_id, prev_id, "FOLLOWS")?;
        }

        // Wire PART_OF edge: run → thread
        write_edge(&storage, run_id, thread_id, "PART_OF")?;

        // Carry forward open questions from prior runs (CARRIED edge)
        let open_q_ids =
            ids_from_index(&storage, QUESTION_THREAD_ID, &Value::U128(thread_id))?;
        for qid in open_q_ids {
            let props = read_node_props(&storage, qid)?;
            let map: std::collections::HashMap<String, Value> = props.into_iter().collect();
            let is_open = map
                .get("status")
                .map(|v| matches!(v, Value::String(s) if s.as_str() == "open"))
                .unwrap_or(false);
            if is_open {
                write_edge(&storage, run_id, qid, "CARRIED")?;
            }
        }

        Ok(Self { storage, thread_id, run_id })
    }

    /// Record a finding produced by this run.
    pub fn record_finding(&self, finding: Finding) -> Result<FindingId, MemoryError> {
        let mut props: Vec<(&str, Value)> = vec![
            ("claim",      Value::String(finding.claim.clone())),
            ("confidence", Value::F32(finding.confidence)),
            ("thread_id",  Value::U128(self.thread_id)),
        ];
        if let Some(eid) = finding.entity_id {
            props.push(("entity_id", Value::U128(eid)));
        }
        if let Some(ref el) = finding.entity_label {
            props.push(("entity_label", Value::String(el.clone())));
        }
        if !finding.metadata.is_empty() {
            props.push(("metadata", Value::Object(finding.metadata.clone())));
        }

        let id = write_node_indexed(
            &self.storage,
            "finding",
            props,
            FINDING_THREAD_ID,
            Value::U128(self.thread_id),
        )?;

        // Link run → finding
        write_edge(&self.storage, self.run_id, id, "PRODUCED")?;

        // Also add to the entity index if entity_id was set
        if let Some(eid) = finding.entity_id {
            add_to_index(&self.storage, FINDING_ENTITY_ID, &Value::U128(eid), id)?;
        }

        Ok(FindingId(id))
    }

    /// Record an open question raised by this run.
    pub fn record_question(
        &self,
        question: &str,
        priority: Priority,
    ) -> Result<QuestionId, MemoryError> {
        let id = write_node_indexed(
            &self.storage,
            "open_question",
            vec![
                ("question",  Value::String(question.to_string())),
                ("priority",  Value::String(priority.as_str().to_string())),
                ("status",    Value::String("open".to_string())),
                ("thread_id", Value::U128(self.thread_id)),
            ],
            QUESTION_THREAD_ID,
            Value::U128(self.thread_id),
        )?;

        // Link run → question
        write_edge(&self.storage, self.run_id, id, "RAISED")?;

        Ok(QuestionId(id))
    }

    /// Answer (close) an existing open question.
    pub fn answer_question(&self, question_id: QuestionId) -> Result<(), MemoryError> {
        let id = question_id.0;
        let props = read_node_props(&self.storage, id)?;
        let mut map: std::collections::HashMap<String, Value> =
            props.into_iter().collect();
        map.insert("status".to_string(), Value::String("answered".to_string()));
        overwrite_node(&self.storage, id, "open_question", map)?;
        Ok(())
    }

    /// Complete the run, writing a summary node.
    pub fn complete(
        &self,
        summary: &str,
        finding_count: u32,
        question_count: u32,
    ) -> Result<(), MemoryError> {
        // Write summary node
        let summary_id = write_node_indexed(
            &self.storage,
            "run_summary",
            vec![
                ("summary",        Value::String(summary.to_string())),
                ("finding_count",  Value::U32(finding_count)),
                ("question_count", Value::U32(question_count)),
                ("run_id",         Value::U128(self.run_id)),
                ("thread_id",      Value::U128(self.thread_id)),
            ],
            SUMMARY_THREAD_ID,
            Value::U128(self.thread_id),
        )?;

        // Link summary → run
        write_edge(&self.storage, summary_id, self.run_id, "SUMMARIZES")?;

        // Update run status to completed
        let props = read_node_props(&self.storage, self.run_id)?;
        let mut map: std::collections::HashMap<String, Value> =
            props.into_iter().collect();
        map.insert("status".to_string(), Value::String("completed".to_string()));
        overwrite_node(&self.storage, self.run_id, "agent_run", map)?;

        Ok(())
    }
}

/// Re-write a node with updated properties (same ID).
/// Uses heed3's `put` which overwrites on same key.
pub(crate) fn overwrite_node(
    storage: &SparrowGraphStorage,
    id: u128,
    label: &str,
    props_map: std::collections::HashMap<String, Value>,
) -> Result<(), MemoryError> {
    let arena = bumpalo::Bump::new();
    let label_ref: &str = arena.alloc_str(label);

    let len = props_map.len();
    let props_vec: Vec<(&str, Value)> = props_map
        .iter()
        .map(|(k, v)| (arena.alloc_str(k) as &str, v.clone()))
        .collect();

    let properties = if len == 0 {
        None
    } else {
        Some(ImmutablePropertiesMap::new(len, props_vec.into_iter(), &arena))
    };

    let node = Node {
        id,
        label: label_ref,
        version: 2,
        properties,
    };

    let bytes = bincode::serialize(&node).map_err(MemoryError::Serialization)?;

    let mut wtxn = storage.graph_env.write_txn().map_err(MemoryError::Heed)?;
    storage
        .nodes_db
        .put(&mut wtxn, &id, &bytes)
        .map_err(MemoryError::Heed)?;
    wtxn.commit().map_err(MemoryError::Heed)?;

    Ok(())
}
