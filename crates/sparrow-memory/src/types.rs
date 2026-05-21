use sparrow_db::protocol::value::Value;
use std::collections::HashMap;

// ── Opaque ID newtypes ────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FindingId(pub(crate) u128);

impl FindingId {
    pub fn as_u128(self) -> u128 { self.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuestionId(pub(crate) u128);

impl QuestionId {
    pub fn as_u128(self) -> u128 { self.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadId(pub(crate) u128);

impl ThreadId {
    pub fn as_u128(self) -> u128 { self.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunId(pub(crate) u128);

impl RunId {
    pub fn as_u128(self) -> u128 { self.0 }
}

// ── Input types (what the agent provides) ────────────────────────────

#[derive(Debug, Clone)]
pub struct Finding {
    /// The finding itself — what the agent concluded. Indexed for recall.
    pub claim: String,
    /// 0.0–1.0
    pub confidence: f32,
    /// Opaque foreign reference into the domain graph (e.g. a sacred_cow node ID).
    pub entity_id: Option<u128>,
    /// Label of the referenced entity, e.g. "sacred_cow", "intervention".
    pub entity_label: Option<String>,
    /// Agent-specific metadata. Stored as Value::Object. Library never inspects it.
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    High,
    Medium,
    Low,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
        }
    }
}

impl TryFrom<&str> for Priority {
    type Error = ();
    fn try_from(s: &str) -> Result<Self, ()> {
        match s {
            "high"   => Ok(Priority::High),
            "medium" => Ok(Priority::Medium),
            "low"    => Ok(Priority::Low),
            _        => Err(()),
        }
    }
}

// ── Stored types (what recall returns) ───────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct StoredFinding {
    pub id: FindingId,
    pub claim: String,
    pub confidence: f32,
    pub entity_id: Option<u128>,
    pub entity_label: Option<String>,
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredSummary {
    pub run_id: RunId,
    pub summary: String,
    pub finding_count: u32,
    pub question_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredQuestion {
    pub id: QuestionId,
    pub question: String,
    pub priority: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub name: String,
    pub goal: String,
    pub status: String,
}

// ── Recall result ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecallResult {
    /// Summaries of the last 3 completed runs, newest first.
    pub recent_summaries: Vec<StoredSummary>,
    /// Top-K findings from the thread, ordered by recency.
    pub relevant_findings: Vec<StoredFinding>,
    /// All open questions for this thread.
    pub open_questions: Vec<StoredQuestion>,
}
