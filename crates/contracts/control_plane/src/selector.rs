//! Trace-selection contracts used by control commands.

use model_core::ids::{TraceId, TraceName};
use model_core::trace::TraceRecord;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceSelector {
    TraceId(TraceId),
    RootPid(u32),
    Tag(String),
    Name(TraceName),
}

impl TraceSelector {
    pub fn matches(&self, trace: &TraceRecord, root_host_pid: Option<u32>) -> bool {
        match self {
            Self::TraceId(expected) => trace.trace_id == *expected,
            Self::RootPid(expected) => root_host_pid == Some(*expected),
            Self::Tag(expected) => trace.tags.contains(expected),
            Self::Name(expected) => trace.display_name == *expected,
        }
    }
}
