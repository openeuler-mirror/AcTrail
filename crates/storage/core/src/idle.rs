//! Historical idle intervals derived on demand from persisted detector alerts.
use std::time::SystemTime;

use model_core::ids::TraceId;

#[derive(Clone, Debug)]
pub struct IdleQuery {
    pub trace_id: TraceId,
    /// Inclusive overlap bounds; candidates can begin before `from`.
    pub from: Option<SystemTime>,
    pub to: Option<SystemTime>,
    /// Durable detected alert ID, in ascending order.
    pub after: Option<u64>,
    pub limit: usize,
    /// Maximum detected rows consumed, at least limit. Each candidate performs
    /// at most one first-detected lookup and one first-recovery lookup by episode.
    pub candidate_limit: usize,
}

#[derive(Clone, Debug)]
pub struct IdleInterval {
    pub id: String,
    pub session_id: String,
    pub task_id: String,
    pub start_time: SystemTime,
    pub end_time: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct IdlePage {
    pub intervals: Vec<IdleInterval>,
    /// A filtered empty page can still have a continuation. This is a live read,
    /// not a snapshot across pages; refresh to observe recovery of earlier items.
    pub next_cursor: Option<u64>,
    pub has_more: bool,
    /// Detected alert rows consumed, including out-of-window and duplicate rows.
    pub scanned: usize,
    pub axis_end: SystemTime,
}

impl IdlePage {
    pub fn empty(axis_end: SystemTime) -> Self {
        Self {
            intervals: Vec::new(),
            next_cursor: None,
            has_more: false,
            scanned: 0,
            axis_end,
        }
    }
}
