use std::sync::Arc;
use sparrow_db::{protocol::value::Value, sparrow_engine::storage_core::SparrowGraphStorage};
use sparrow_db::sparrow_engine::storage_core::storage_methods::StorageMethods;

use crate::{
    error::MemoryError,
    graph::{ids_from_index, read_node_props, write_node},
    indices::{FINDING_ENTITY_ID, FINDING_THREAD_ID},
    run::RunHandle,
    types::{FindingId, StoredFinding},
};

pub struct ThreadHandle {
    pub(crate) storage: Arc<SparrowGraphStorage>,
    pub(crate) id: u128,
}

impl ThreadHandle {
    pub fn thread_id(&self) -> u128 {
        self.id
    }

    pub fn get_or_create(
        storage: Arc<SparrowGraphStorage>,
        agent: &str,
        name: &str,
        goal: &str,
    ) -> Result<Self, MemoryError> {
        // Scan existing nodes looking for a matching thread
        let found_id: Option<u128> = {
            let rtxn = storage.graph_env.read_txn().map_err(MemoryError::Heed)?;
            let arena = bumpalo::Bump::new();
            let mut found = None;
            for item in storage.nodes_db.iter(&rtxn).map_err(MemoryError::Heed)? {
                let (node_id, _) = item.map_err(MemoryError::Heed)?;
                let node = match storage.get_node(&rtxn, node_id, &arena) {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                if node.label != "research_thread" {
                    continue;
                }
                let agent_match = node
                    .get_property("agent_name")
                    .map(|v| matches!(v, Value::String(s) if s.as_str() == agent))
                    .unwrap_or(false);
                let name_match = node
                    .get_property("name")
                    .map(|v| matches!(v, Value::String(s) if s.as_str() == name))
                    .unwrap_or(false);
                if agent_match && name_match {
                    found = Some(node_id);
                    break;
                }
            }
            found
        }; // rtxn dropped here

        if let Some(node_id) = found_id {
            return Ok(Self { storage, id: node_id });
        }

        let id = write_node(&storage, "research_thread", vec![
            ("name",       Value::String(name.to_string())),
            ("goal",       Value::String(goal.to_string())),
            ("agent_name", Value::String(agent.to_string())),
            ("status",     Value::String("active".to_string())),
        ])?;

        Ok(Self { storage, id })
    }

    pub fn start_run(&self) -> Result<RunHandle, MemoryError> {
        RunHandle::create(Arc::clone(&self.storage), self.id)
    }

    pub fn recall(&self, _query: &str) -> Result<crate::types::RecallResult, MemoryError> {
        crate::recall::build_recall(Arc::clone(&self.storage), self.id)
    }

    /// Count distinct entity IDs among findings for this thread filtered by entity_label.
    pub fn count_distinct(&self, entity_label: &str) -> Result<usize, MemoryError> {
        let finding_ids =
            ids_from_index(&self.storage, FINDING_THREAD_ID, &Value::U128(self.id))?;
        let mut distinct = std::collections::HashSet::new();
        for fid in finding_ids {
            let props = read_node_props(&self.storage, fid)?;
            let map: std::collections::HashMap<String, Value> =
                props.into_iter().collect();
            let el_match = map
                .get("entity_label")
                .map(|v| matches!(v, Value::String(s) if s.as_str() == entity_label))
                .unwrap_or(false);
            if !el_match {
                continue;
            }
            if let Some(Value::U128(eid)) = map.get("entity_id") {
                distinct.insert(*eid);
            }
        }
        Ok(distinct.len())
    }

    /// Return all findings from this thread that reference the given entity_id.
    pub fn findings_for_entity(&self, entity_id: u128) -> Result<Vec<StoredFinding>, MemoryError> {
        let all_ids =
            ids_from_index(&self.storage, FINDING_ENTITY_ID, &Value::U128(entity_id))?;
        let mut findings = Vec::new();
        for fid in all_ids {
            let props = read_node_props(&self.storage, fid)?;
            let map: std::collections::HashMap<String, Value> =
                props.into_iter().collect();
            // Only include findings from this thread
            let in_thread = map
                .get("thread_id")
                .map(|v| matches!(v, Value::U128(tid) if *tid == self.id))
                .unwrap_or(false);
            if !in_thread {
                continue;
            }
            findings.push(props_to_stored_finding(fid, map));
        }
        Ok(findings)
    }
}

pub(crate) fn props_to_stored_finding(
    id: u128,
    props: std::collections::HashMap<String, Value>,
) -> StoredFinding {
    StoredFinding {
        id: FindingId(id),
        claim: props
            .get("claim")
            .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
            .unwrap_or_default(),
        confidence: props
            .get("confidence")
            .and_then(|v| if let Value::F32(f) = v { Some(*f) } else { None })
            .unwrap_or(0.0),
        entity_id: props
            .get("entity_id")
            .and_then(|v| if let Value::U128(id) = v { Some(*id) } else { None }),
        entity_label: props
            .get("entity_label")
            .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None }),
        metadata: props
            .get("metadata")
            .and_then(|v| if let Value::Object(m) = v { Some(m.clone()) } else { None })
            .unwrap_or_default(),
    }
}
