/// Secondary index for finding nodes → look up by thread_id
pub const FINDING_THREAD_ID: &str = "finding:thread_id";
/// Secondary index for finding nodes → look up by entity_id (for count_distinct)
pub const FINDING_ENTITY_ID: &str = "finding:entity_id";
/// Secondary index for open_question nodes → look up by thread_id
pub const QUESTION_THREAD_ID: &str = "question:thread_id";
/// Secondary index for open_question nodes → look up by status (fast open-question scan)
pub const QUESTION_STATUS: &str = "question:status";
/// Secondary index for agent_run nodes → look up by thread_id
pub const RUN_THREAD_ID: &str = "run:thread_id";
/// Secondary index for run_summary nodes → look up by thread_id
pub const SUMMARY_THREAD_ID: &str = "summary:thread_id";

/// All index names — passed to Config on MemoryStore::open
pub const ALL_INDICES: &[&str] = &[
    FINDING_THREAD_ID,
    FINDING_ENTITY_ID,
    QUESTION_THREAD_ID,
    QUESTION_STATUS,
    RUN_THREAD_ID,
    SUMMARY_THREAD_ID,
];
