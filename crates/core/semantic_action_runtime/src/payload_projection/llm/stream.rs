//! Plaintext payload stream indexing for LLM projection.

use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct PayloadStreamGroupKey {
    pub(crate) trace_id: TraceId,
    pub(crate) process: ProcessIdentity,
    pub(crate) stream_key: String,
}

impl PayloadStreamGroupKey {
    pub(crate) fn from_segment(segment: &PayloadSegment) -> Self {
        Self {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
        }
    }
}
