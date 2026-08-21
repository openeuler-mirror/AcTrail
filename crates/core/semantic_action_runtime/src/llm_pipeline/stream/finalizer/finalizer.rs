//! Unified logical-stream completion and partial finalization.

use std::time::SystemTime;

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, attr_keys as attrs,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) enum StreamFinalizationReason {
    PeerClosed,
    TraceClosed,
    ConfirmedGap,
    OperationIncomplete,
    ProtocolDecodeFailed,
    Http2StreamReset,
    BufferBytesExceeded,
    SegmentRangesExceeded,
}

/// Applies terminal status consistently after a stream component has
/// materialized a partial LLM request or response from retained bytes.
pub(in crate::llm_pipeline) struct ResponseFinalizer;

impl ResponseFinalizer {
    pub(in crate::llm_pipeline) fn finalize_partial(
        action: &mut SemanticAction,
        reason: StreamFinalizationReason,
        observed_at: SystemTime,
    ) {
        action.status = SemanticActionStatus::Error;
        action.completeness = SemanticActionCompleteness::Partial;
        action.end_time = Some(observed_at);
        if reason == StreamFinalizationReason::TraceClosed {
            action.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                "true".to_string(),
            );
        }
    }
}
