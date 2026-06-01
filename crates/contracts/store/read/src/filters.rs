//! Trace-read filter contracts.

use std::collections::BTreeSet;

use model_core::ids::{TraceId, TraceName};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TraceFilter {
    pub trace_ids: BTreeSet<TraceId>,
    pub root_pids: BTreeSet<u32>,
    pub tags: BTreeSet<String>,
    pub names: BTreeSet<TraceName>,
}
