//! Unified logical-stream completion and partial finalization.

use std::time::SystemTime;

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, attr_keys as attrs,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) enum StreamFinalizationReason {
    PeerClosed,
    TraceClosed,
    CapturePolicyLimited,
    ConfirmedGap,
    OperationIncomplete,
    ProtocolDecodeFailed,
    Http2StreamReset,
    BufferBytesExceeded,
    SegmentRangesExceeded,
}

impl StreamFinalizationReason {
    pub(in crate::llm_pipeline) const fn response_reusable(self) -> bool {
        matches!(self, Self::CapturePolicyLimited)
    }

    const fn terminal_state(self) -> (SemanticActionStatus, SemanticActionCompleteness) {
        match self {
            Self::CapturePolicyLimited => (
                SemanticActionStatus::Success,
                SemanticActionCompleteness::CaptureLimited,
            ),
            _ => (
                SemanticActionStatus::Error,
                SemanticActionCompleteness::Partial,
            ),
        }
    }
}

/// Applies terminal status consistently after a stream component has
/// materialized a partial LLM request or response from retained bytes.
pub(in crate::llm_pipeline) struct ResponseFinalizer;

impl ResponseFinalizer {
    /// Closes a request-side call whose response was not observed.
    ///
    /// A stream boundary is evidence about observation completeness, not the
    /// model outcome. In particular, a late trace close must not manufacture
    /// either an error or a response end time for an otherwise successful
    /// request observation.
    pub(in crate::llm_pipeline) fn finalize_response_unobserved(
        action: &mut SemanticAction,
        reason: StreamFinalizationReason,
    ) {
        action.end_time = None;
        action.status = SemanticActionStatus::Unknown;
        action.completeness = SemanticActionCompleteness::Partial;
        if reason == StreamFinalizationReason::TraceClosed {
            action.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                "true".to_string(),
            );
        }
    }

    pub(in crate::llm_pipeline) fn finalize_incomplete(
        action: &mut SemanticAction,
        reason: StreamFinalizationReason,
        observed_at: SystemTime,
    ) {
        action.end_time = Some(observed_at);
        let (status, completeness) = reason.terminal_state();
        action.status = status;
        action.completeness = completeness;
        if reason == StreamFinalizationReason::TraceClosed {
            action.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                "true".to_string(),
            );
        }
    }
}
