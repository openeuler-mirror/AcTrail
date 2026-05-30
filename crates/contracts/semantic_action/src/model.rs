//! Cross-layer semantic action records.

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionKind {
    ProcessExec,
    FileModify,
    HttpMessage,
    LlmRequest,
    EnforcementDecision,
    ProcessForkAttempt,
    AgentInvocation,
}

impl SemanticActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProcessExec => "process.exec",
            Self::FileModify => "file.modify",
            Self::HttpMessage => "http.message",
            Self::LlmRequest => "llm.request",
            Self::EnforcementDecision => "enforcement.decision",
            Self::ProcessForkAttempt => "process.fork_attempt",
            Self::AgentInvocation => "agent.invocation",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "process.exec" => Some(Self::ProcessExec),
            "file.modify" => Some(Self::FileModify),
            "http.message" => Some(Self::HttpMessage),
            "llm.request" => Some(Self::LlmRequest),
            "enforcement.decision" => Some(Self::EnforcementDecision),
            "process.fork_attempt" => Some(Self::ProcessForkAttempt),
            "agent.invocation" => Some(Self::AgentInvocation),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionStatus {
    InProgress,
    Success,
    Error,
    Unknown,
}

impl SemanticActionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Success => "success",
            Self::Error => "error",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "in_progress" => Some(Self::InProgress),
            "success" => Some(Self::Success),
            "error" => Some(Self::Error),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionCompleteness {
    Complete,
    Partial,
    Inferred,
}

impl SemanticActionCompleteness {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Inferred => "inferred",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "complete" => Some(Self::Complete),
            "partial" => Some(Self::Partial),
            "inferred" => Some(Self::Inferred),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticEvidenceKind {
    Event,
    PayloadSegment,
}

impl SemanticEvidenceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::PayloadSegment => "payload_segment",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "event" => Some(Self::Event),
            "payload_segment" => Some(Self::PayloadSegment),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticEvidence {
    pub kind: SemanticEvidenceKind,
    pub id: u64,
    pub role: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticAction {
    pub action_id: String,
    pub trace_id: TraceId,
    pub kind: SemanticActionKind,
    pub title: String,
    pub start_time: SystemTime,
    pub end_time: Option<SystemTime>,
    pub process: ProcessIdentity,
    pub status: SemanticActionStatus,
    pub completeness: SemanticActionCompleteness,
    pub confidence_millis: Option<u16>,
    pub attributes: BTreeMap<String, String>,
    pub evidence: Vec<SemanticEvidence>,
}
