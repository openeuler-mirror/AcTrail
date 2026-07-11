//! Raw payload segment contracts emitted by payload-capable collectors.

use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessObservation;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawPayloadSegment {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessObservation,
    pub source_boundary: PayloadSourceBoundary,
    pub content_state: PayloadContentState,
    pub direction: PayloadDirection,
    pub stream_key: PayloadStreamKey,
    pub sequence: u64,
    pub original_size: u64,
    pub captured_size: u64,
    pub operation_id: u64,
    pub operation_offset: u64,
    pub operation_original_size: u64,
    pub operation_captured_size: u64,
    pub operation_completion_state: PayloadOperationCompletionState,
    pub truncation: PayloadTruncationState,
    pub library: String,
    pub symbol: String,
    pub protocol_hint: Option<String>,
    pub bytes: Vec<u8>,
}
