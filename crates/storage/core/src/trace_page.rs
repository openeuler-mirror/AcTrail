//! Trace overview counts and numbered metadata pages.
use std::time::SystemTime;

use model_core::{ids::TraceId, trace::TraceRecord};

#[derive(Clone, Debug)]
pub struct TracePageQuery {
    pub page: usize,
    pub limit: usize,
    /// Exact persisted display name; combined with every other supplied filter.
    pub name: Option<String>,
    pub trace_id: Option<TraceId>,
    /// Inclusive creation-time bounds, independent of execution start/end times.
    pub start_time: Option<SystemTime>,
    pub end_time: Option<SystemTime>,
}

#[derive(Clone, Debug, Default)]
pub struct TracePage {
    pub traces: Vec<TraceRecord>,
    pub page: usize,
    pub total: u64,
    pub total_pages: u64,
}

/// Exact persisted row counts; processes count trace memberships, not process events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TraceCounts {
    pub events: u64,
    pub payloads: u64,
    pub actions: u64,
    pub processes: u64,
}
