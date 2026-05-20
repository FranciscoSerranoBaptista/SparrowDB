use std::sync::Arc;
use sparrow_db::{
    protocol::value::Value,
    sparrow_engine::storage_core::SparrowGraphStorage,
    utils::{items::Node, properties::ImmutablePropertiesMap},
};

use crate::{
    error::MemoryError,
    graph::{add_to_index, ids_from_index, out_neighbors, read_node_props, remove_from_index, write_edge, write_node_indexed},
    indices::{FINDING_ENTITY_ID, FINDING_THREAD_ID, QUESTION_STATUS, QUESTION_THREAD_ID, RUN_THREAD_ID, SUMMARY_THREAD_ID},
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

    pub fn create(storage: Arc<SparrowGraphStorage>, thread_id: u128) -> Result<Self, MemoryError> {
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

        // Wire FOLLOWS edge to the most recent *completed* run — skip orphaned/interrupted runs
        let prev_runs = ids_from_index(&storage, RUN_THREAD_ID, &Value::U128(thread_id))?;
        let prev = prev_runs.iter().copied()
            .filter(|&id| {
                if id == run_id { return false; }
                let props = read_node_props(&storage, id).unwrap_or_default();
                let map: std::collections::HashMap<String, Value> = props.into_iter().collect();
                matches!(map.get("status"), Some(Value::String(s)) if s.as_str() == "completed")
            })
            .max();
        if let Some(prev_id) = prev {
            write_edge(&storage, run_id, prev_id, "FOLLOWS")?;
        }

        write_edge(&storage, run_id, thread_id, "PART_OF")?;

        // Carry forward open questions from this thread
        let open_q_ids = ids_from_index(&storage, QUESTION_THREAD_ID, &Value::U128(thread_id))?;
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

        write_edge(&self.storage, self.run_id, id, "PRODUCED")?;

        if let Some(eid) = finding.entity_id {
            add_to_index(&self.storage, FINDING_ENTITY_ID, &Value::U128(eid), id)?;
        }

        Ok(FindingId(id))
    }

    /// Raise an open question from this run.
    pub fn raise_question(&self, question: &str, priority: Priority) -> Result<QuestionId, MemoryError> {
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

        add_to_index(&self.storage, QUESTION_STATUS, &Value::String("open".to_string()), id)?;

        write_edge(&self.storage, self.run_id, id, "RAISED")?;

        Ok(QuestionId(id))
    }

    /// Mark an open question resolved, linking the finding that answers it.
    pub fn answer_question(&self, question_id: QuestionId, finding_id: FindingId) -> Result<(), MemoryError> {
        let qid = question_id.0;
        let fid = finding_id.0;
        let props = read_node_props(&self.storage, qid)?;
        let mut map: std::collections::HashMap<String, Value> = props.into_iter().collect();
        map.insert("status".to_string(), Value::String("resolved".to_string()));
        overwrite_node(&self.storage, qid, "open_question", map)?;
        // Remove the now-stale "open" entry from the status index so callers of QUESTION_STATUS
        // don't see resolved questions as open.
        remove_from_index(&self.storage, QUESTION_STATUS, &Value::String("open".to_string()), qid)?;
        write_edge(&self.storage, fid, qid, "ANSWERS")?;
        Ok(())
    }

    /// Complete the run: write a summary node, derive counts from graph, update status.
    pub fn complete(self, summary: &str) -> Result<(), MemoryError> {
        let finding_count = out_neighbors(&self.storage, self.run_id, "PRODUCED")?.len() as u32;
        let question_count = out_neighbors(&self.storage, self.run_id, "RAISED")?.len() as u32;

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

        write_edge(&self.storage, summary_id, self.run_id, "SUMMARIZES")?;

        let props = read_node_props(&self.storage, self.run_id)?;
        let mut map: std::collections::HashMap<String, Value> = props.into_iter().collect();
        map.insert("status".to_string(), Value::String("completed".to_string()));
        overwrite_node(&self.storage, self.run_id, "agent_run", map)?;

        Ok(())
    }

    /// Mark the run interrupted; open questions are carried forward on next start_run.
    pub fn interrupt(self) -> Result<(), MemoryError> {
        let props = read_node_props(&self.storage, self.run_id)?;
        let mut map: std::collections::HashMap<String, Value> = props.into_iter().collect();
        map.insert("status".to_string(), Value::String("interrupted".to_string()));
        overwrite_node(&self.storage, self.run_id, "agent_run", map)?;
        Ok(())
    }
}

/// Re-write a node with updated properties (same ID). Uses plain `put` which overwrites on same key.
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
