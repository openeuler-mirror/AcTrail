//! Explicit changes to an action's runtime-owned state.

use std::time::SystemTime;

use model_core::{ids::TraceId, process::ProcessIdentity};

use crate::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus, SemanticEvidence,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionUpdate {
    pub action_id: String,
    pub trace_id: TraceId,
    pub kind: SemanticActionKind,
    pub process: ProcessIdentity,
    pub change: SemanticActionChange,
    pub evidence: Vec<SemanticEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticActionChange {
    Lifecycle {
        end_time: Option<SystemTime>,
        status: SemanticActionStatus,
        completeness: SemanticActionCompleteness,
        finalization_reason: Option<SemanticActionFinalizationReason>,
    },
    LlmResponseFailure {
        title: String,
        end_time: Option<SystemTime>,
        body_format: String,
        http_status_code: Option<u16>,
        http_reason: Option<String>,
    },
    CommandClassification {
        kind: SemanticCommandKind,
    },
    CommandToolName {
        tool_name: String,
    },
    ToolResultBinding {
        state: SemanticToolResultBinding,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionFinalizationReason {
    TraceClosed,
    CapacityEvicted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticToolResultBinding {
    MissingId,
    Unmatched,
    Bound,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticCommandKind {
    Agent,
    Mcp,
    Command,
}
