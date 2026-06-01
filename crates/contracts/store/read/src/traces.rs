//! Trace-read contracts.

use model_core::ids::TraceId;
use model_core::trace::TraceRecord;

use crate::ReadError;
use crate::filters::TraceFilter;

pub trait TraceReadStore {
    fn get_trace(&self, trace_id: TraceId) -> Result<Option<TraceRecord>, ReadError>;
    fn list_traces(&self, filter: &TraceFilter) -> Result<Vec<TraceRecord>, ReadError>;
}
