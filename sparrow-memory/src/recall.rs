use std::sync::Arc;
use sparrow_db::{protocol::value::Value, sparrow_engine::storage_core::SparrowGraphStorage};

use crate::{
    error::MemoryError,
    graph::{ids_from_index, read_node_props},
    indices::{FINDING_THREAD_ID, QUESTION_THREAD_ID, SUMMARY_THREAD_ID},
    thread::props_to_stored_finding,
    types::{QuestionId, RecallResult, RunId, StoredQuestion, StoredSummary},
};

/// Build the full recall result for a thread.
pub fn build_recall(
    storage: Arc<SparrowGraphStorage>,
    thread_id: u128,
) -> Result<RecallResult, MemoryError> {
    let recent_summaries = build_summaries(&storage, thread_id)?;
    let relevant_findings = build_findings(&storage, thread_id)?;
    let open_questions = build_open_questions(&storage, thread_id)?;

    Ok(RecallResult {
        recent_summaries,
        relevant_findings,
        open_questions,
    })
}

fn build_summaries(
    storage: &SparrowGraphStorage,
    thread_id: u128,
) -> Result<Vec<StoredSummary>, MemoryError> {
    let ids = ids_from_index(storage, SUMMARY_THREAD_ID, &Value::U128(thread_id))?;

    let mut summaries: Vec<(u128, StoredSummary)> = Vec::new();
    for sid in ids {
        let props = read_node_props(storage, sid)?;
        let map: std::collections::HashMap<String, Value> = props.into_iter().collect();

        let run_id = map
            .get("run_id")
            .and_then(|v| if let Value::U128(id) = v { Some(*id) } else { None })
            .unwrap_or(0);
        let summary_text = map
            .get("summary")
            .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
            .unwrap_or_default();
        let finding_count = map
            .get("finding_count")
            .and_then(|v| if let Value::U32(n) = v { Some(*n) } else { None })
            .unwrap_or(0);
        let question_count = map
            .get("question_count")
            .and_then(|v| if let Value::U32(n) = v { Some(*n) } else { None })
            .unwrap_or(0);

        summaries.push((sid, StoredSummary {
            run_id: RunId(run_id),
            summary: summary_text,
            finding_count,
            question_count,
        }));
    }

    // Sort by summary node ID descending (newest first — v6 UUIDs are time-ordered)
    summaries.sort_by(|a, b| b.0.cmp(&a.0));
    summaries.truncate(3);

    Ok(summaries.into_iter().map(|(_, s)| s).collect())
}

fn build_findings(
    storage: &SparrowGraphStorage,
    thread_id: u128,
) -> Result<Vec<crate::types::StoredFinding>, MemoryError> {
    let ids = ids_from_index(storage, FINDING_THREAD_ID, &Value::U128(thread_id))?;

    let mut findings: Vec<(u128, crate::types::StoredFinding)> = Vec::new();
    for fid in ids {
        let props = read_node_props(storage, fid)?;
        let map: std::collections::HashMap<String, Value> = props.into_iter().collect();
        findings.push((fid, props_to_stored_finding(fid, map)));
    }

    // Sort by finding node ID descending (newest first)
    findings.sort_by(|a, b| b.0.cmp(&a.0));
    findings.truncate(20);

    Ok(findings.into_iter().map(|(_, f)| f).collect())
}

fn build_open_questions(
    storage: &SparrowGraphStorage,
    thread_id: u128,
) -> Result<Vec<StoredQuestion>, MemoryError> {
    let ids = ids_from_index(storage, QUESTION_THREAD_ID, &Value::U128(thread_id))?;

    let mut questions = Vec::new();
    for qid in ids {
        let props = read_node_props(storage, qid)?;
        let map: std::collections::HashMap<String, Value> = props.into_iter().collect();

        let is_open = map
            .get("status")
            .map(|v| matches!(v, Value::String(s) if s.as_str() == "open"))
            .unwrap_or(false);
        if !is_open {
            continue;
        }

        let question_text = map
            .get("question")
            .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
            .unwrap_or_default();
        let priority = map
            .get("priority")
            .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
            .unwrap_or_default();

        questions.push(StoredQuestion {
            id: QuestionId(qid),
            question: question_text,
            priority,
        });
    }

    Ok(questions)
}
