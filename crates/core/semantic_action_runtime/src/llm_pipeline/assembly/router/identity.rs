//! Plaintext payload stream indexing for LLM projection.

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadDirection, PayloadSegment, PayloadSourceBoundary, PayloadStreamIdentity,
};
use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PayloadStreamGroupKey {
    pub(crate) trace_id: TraceId,
    pub(crate) process: ProcessIdentity,
    pub(crate) stream_key: String,
    pub(crate) source_boundary: PayloadSourceBoundary,
}

impl Ord for PayloadStreamGroupKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.trace_id,
            self.process,
            self.source_boundary as u8,
            &self.stream_key,
        )
            .cmp(&(
                other.trace_id,
                other.process,
                other.source_boundary as u8,
                &other.stream_key,
            ))
    }
}

impl PartialOrd for PayloadStreamGroupKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PayloadStreamGroupKey {
    pub(crate) fn from_segment(segment: &PayloadSegment) -> Self {
        Self {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
            source_boundary: segment.source_boundary,
        }
    }

    pub(in crate::llm_pipeline) fn from_identity(identity: &PayloadStreamIdentity) -> Self {
        Self {
            trace_id: identity.trace_id,
            process: identity.process,
            stream_key: identity.stream_key.to_string(),
            source_boundary: identity.source_boundary,
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
