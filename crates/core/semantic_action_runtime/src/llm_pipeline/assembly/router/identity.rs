//! Plaintext payload stream indexing for LLM projection.

use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment};
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::llm_pipeline) struct LiveStreamKey {
    pub(in crate::llm_pipeline) group: PayloadStreamGroupKey,
    pub(in crate::llm_pipeline) direction: LiveStreamDirection,
}

impl LiveStreamKey {
    pub(in crate::llm_pipeline) fn from_segment(segment: &PayloadSegment) -> Self {
        Self {
            group: PayloadStreamGroupKey::from_segment(segment),
            direction: LiveStreamDirection::from(segment.direction),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::llm_pipeline) enum LiveStreamDirection {
    Outbound,
    Inbound,
}

impl From<PayloadDirection> for LiveStreamDirection {
    fn from(direction: PayloadDirection) -> Self {
        match direction {
            PayloadDirection::Outbound => Self::Outbound,
            PayloadDirection::Inbound => Self::Inbound,
        }
    }
}
