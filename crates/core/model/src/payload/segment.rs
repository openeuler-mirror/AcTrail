//! Persisted payload segment model.

use std::fmt;
use std::time::SystemTime;

use crate::ids::TraceId;
use crate::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PayloadSegmentId(u64);

impl PayloadSegmentId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PayloadSegmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "payload-{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PayloadStreamKey(String);

impl PayloadStreamKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PayloadStreamKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadSourceBoundary {
    TlsUserSpace,
    Syscall,
    Stdio,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadContentState {
    Plaintext,
    Ciphertext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadDirection {
    Outbound,
    Inbound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadRedactionState {
    NotRequired,
    Redacted,
    Unredacted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTruncationState {
    Complete,
    Truncated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadOperationCompletionState {
    Unknown,
    Success,
    Partial,
    Failed,
}

impl PayloadOperationCompletionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadSegment {
    pub segment_id: PayloadSegmentId,
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
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
    pub redaction: PayloadRedactionState,
    pub library: String,
    pub symbol: String,
    pub protocol_hint: Option<String>,
    pub bytes: Vec<u8>,
}
