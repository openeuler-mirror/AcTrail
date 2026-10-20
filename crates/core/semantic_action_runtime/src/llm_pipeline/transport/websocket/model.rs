//! Native WebSocket message identity and capture lifecycle.
use model_core::payload::{PayloadDirection, PayloadStreamIdentity};
use semantic_action::SemanticEvidence;
use std::time::SystemTime;

#[derive(Clone)]
pub(in crate::llm_pipeline) struct WebSocketMessageHead {
    pub(in crate::llm_pipeline) message_id: String,
    pub(in crate::llm_pipeline) connection_id: String,
    pub(in crate::llm_pipeline) path: String,
    pub(in crate::llm_pipeline) identity: PayloadStreamIdentity,
    pub(in crate::llm_pipeline) direction: PayloadDirection,
    pub(in crate::llm_pipeline) opcode: u8,
    pub(in crate::llm_pipeline) start_at: SystemTime,
    pub(in crate::llm_pipeline) evidence: Vec<SemanticEvidence>,
}
#[derive(Clone, Copy, Debug)]
pub(in crate::llm_pipeline) enum WebSocketAbortReason {
    CaptureGap,
    CaptureIncomplete,
    CapturePolicyLimited,
    DecodeFailure,
    CapacityExceeded,
    PeerClosed,
    TraceClosed,
}
pub(in crate::llm_pipeline) enum WebSocketMessageEvent {
    Start(WebSocketMessageHead),
    Complete {
        head: WebSocketMessageHead,
        bytes: Vec<u8>,
        evidence: Vec<SemanticEvidence>,
        observed_at: SystemTime,
    },
    Abort {
        head: WebSocketMessageHead,
        evidence: Vec<SemanticEvidence>,
        observed_at: SystemTime,
        reason: WebSocketAbortReason,
    },
}
#[derive(Clone, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct WebSocketConnectionKey(
    pub(in crate::llm_pipeline) PayloadStreamIdentity,
);
impl Ord for WebSocketConnectionKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.0.trace_id,
            self.0.process,
            self.0.source_boundary as u8,
            &self.0.stream_key,
        )
            .cmp(&(
                other.0.trace_id,
                other.0.process,
                other.0.source_boundary as u8,
                &other.0.stream_key,
            ))
    }
}
impl PartialOrd for WebSocketConnectionKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
