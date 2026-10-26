//! Bounded timeline reads. Continuation tracks consumed candidates, including filtered rows.
//!
//! Rows contain hot lifecycle/classification fields; attributes exclude cold payloads
//! and evidence. Unknown LLM calls inspect at most two direct response edges and one
//! response row per candidate/context action to derive a completed response outcome.
//! Reads observe live persisted state and do not promise a multi-request snapshot.
use std::time::SystemTime;

use model_core::{ids::TraceId, process::ProcessIdentity};
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionLink};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineCursor {
    pub start_time: SystemTime,
    /// Storage identity used only as the tie breaker; never a process or display ID.
    pub action_key: i64,
}

/// First and last action start times across a trace, independent of view filters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimelineBounds {
    pub first_start_time: Option<SystemTime>,
    pub last_start_time: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct TimelineQuery {
    pub trace_id: TraceId,
    pub kinds: Vec<SemanticActionKind>,
    pub process: Option<ProcessIdentity>,
    /// Inclusive lower bound for action start times.
    pub start_time: Option<SystemTime>,
    /// Inclusive upper bound for action start times.
    pub end_time: Option<SystemTime>,
    pub after: Option<TimelineCursor>,
    pub limit: usize,
    /// Total index candidates across all kinds. Must cover each distinct kind.
    /// Both time bounds are applied during the index seek. A full last range can
    /// produce one final empty page before exhaustion is known.
    pub candidate_limit: usize,
    /// Total direct edge candidates, including invalid edges and repeated endpoints.
    pub link_limit: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TimelinePage {
    pub actions: Vec<SemanticAction>,
    /// Direct endpoints only; these may fall outside the requested filter/window.
    pub context_actions: Vec<SemanticAction>,
    pub links: Vec<SemanticActionLink>,
    /// May be present on an empty page. Consumers must explicitly request more.
    pub next_cursor: Option<TimelineCursor>,
    /// Index rows read, including prefetched rows after the safe cursor boundary.
    pub scanned: usize,
    /// Direct relationship scanning or endpoint resolution was incomplete.
    pub links_truncated: bool,
}
