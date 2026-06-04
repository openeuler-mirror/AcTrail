//! Cross-layer semantic action records.

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionKind {
    ProcessExec,
    FileModify,
    FileWrite,
    HttpMessage,
    LlmRequest,
    EnforcementDecision,
    ProcessForkAttempt,
    AgentInvocation,
    CommandInvocation,
}

impl SemanticActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProcessExec => "process.exec",
            Self::FileModify => "file.modify",
            Self::FileWrite => "file.write",
            Self::HttpMessage => "http.message",
            Self::LlmRequest => "llm.request",
            Self::EnforcementDecision => "enforcement.decision",
            Self::ProcessForkAttempt => "process.fork_attempt",
            Self::AgentInvocation => "agent.invocation",
            Self::CommandInvocation => "command.invocation",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "process.exec" => Some(Self::ProcessExec),
            "file.modify" => Some(Self::FileModify),
            "file.write" => Some(Self::FileWrite),
            "http.message" => Some(Self::HttpMessage),
            "llm.request" => Some(Self::LlmRequest),
            "enforcement.decision" => Some(Self::EnforcementDecision),
            "process.fork_attempt" => Some(Self::ProcessForkAttempt),
            "agent.invocation" => Some(Self::AgentInvocation),
            "command.invocation" => Some(Self::CommandInvocation),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionLinkRole {
    AgentPerformedAction,
    CommandContainsProcessExec,
    FileWriteContainsFileEvent,
    AgentInvocationExec,
    AgentInvocationChildLlmRequest,
    LlmRequestHttpMessage,
}

impl SemanticActionLinkRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentPerformedAction => "agent.performed_action",
            Self::CommandContainsProcessExec => "command.contains_process_exec",
            Self::FileWriteContainsFileEvent => "file.write.contains_file_event",
            Self::AgentInvocationExec => "agent.invocation.exec",
            Self::AgentInvocationChildLlmRequest => "agent.invocation.child_llm_request",
            Self::LlmRequestHttpMessage => "llm.request.http_message",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "agent.performed_action" => Some(Self::AgentPerformedAction),
            "command.contains_process_exec" => Some(Self::CommandContainsProcessExec),
            "file.write.contains_file_event" => Some(Self::FileWriteContainsFileEvent),
            "agent.invocation.exec" => Some(Self::AgentInvocationExec),
            "agent.invocation.child_llm_request" => Some(Self::AgentInvocationChildLlmRequest),
            "llm.request.http_message" => Some(Self::LlmRequestHttpMessage),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionLinkConfidence {
    Observed,
    Derived,
}

impl SemanticActionLinkConfidence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Derived => "derived",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "observed" => Some(Self::Observed),
            "derived" => Some(Self::Derived),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionLink {
    pub trace_id: TraceId,
    pub parent_action_id: String,
    pub child_action_id: String,
    pub role: SemanticActionLinkRole,
    pub confidence: SemanticActionLinkConfidence,
    pub evidence: Vec<SemanticEvidence>,
    pub attributes: BTreeMap<String, String>,
}
