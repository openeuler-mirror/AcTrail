//! Protocol observations shared by semantic and application projections.

use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegmentId, PayloadSourceBoundary};
use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Http1MessageSource {
    pub trace_id: TraceId,
    pub process: ProcessIdentity,
    pub source_boundary: PayloadSourceBoundary,
    pub stream_key: String,
    pub direction: PayloadDirection,
    pub observed_at: SystemTime,
    pub segment_id: PayloadSegmentId,
    pub payload_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Http1MessageHead {
    pub message_id: String,
    pub source: Http1MessageSource,
    pub protocol: &'static str,
    pub method: Option<String>,
    pub authority: Option<String>,
    pub path: Option<String>,
    pub status_code: Option<String>,
    pub reason: Option<String>,
    pub headers_text: String,
    pub declared_body_len: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Http1MessageEnd {
    pub message_id: String,
    pub observed_at: SystemTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Http1AbortReason {
    CaptureGap,
    CaptureIncomplete,
    CapturePolicyLimited,
    DecodeFailure,
    CapacityExceeded,
    PeerClosed,
    Http2StreamReset,
    TraceClosed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Http1MessageEvent {
    Head(Http1MessageHead),
    Body {
        message_id: String,
        bytes: Vec<u8>,
    },
    End(Http1MessageEnd),
    Abort {
        message_id: String,
        reason: Http1AbortReason,
        observed_at: SystemTime,
    },
}

/// A frame decoded once by the transport owner. Message identity is directional.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Http2FrameObservation {
    pub source: Http1MessageSource,
    pub observed_at: SystemTime,
    pub payload_segment_ids: Vec<PayloadSegmentId>,
    pub frame_id: String,
    pub message_id: Option<String>,
    pub frame_type: u8,
    pub flags: u8,
    pub stream_id: u32,
    pub payload: std::sync::Arc<Vec<u8>>,
    pub connection_preface: bool,
}
