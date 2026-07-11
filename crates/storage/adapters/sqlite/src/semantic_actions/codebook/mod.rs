//! Enum codebook for semantic action SQLite storage.

mod current;
mod error;
pub(in crate::semantic_actions) mod sqlite;

use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkConfidence,
    SemanticActionLinkRole, SemanticActionStatus, SemanticEvidenceKind,
};

pub(crate) use error::CodebookError;
use error::validate_unique;

pub(crate) const CURRENT_SCHEMA_VERSION: i32 = current::SCHEMA_VERSION;

pub(crate) fn current() -> &'static SemanticActionCodebook {
    current::CODEBOOK
}

pub(crate) fn for_schema_version(
    schema_version: i32,
) -> Result<&'static SemanticActionCodebook, CodebookError> {
    match schema_version {
        CURRENT_SCHEMA_VERSION => Ok(current::CODEBOOK),
        _ => Err(CodebookError::new(
            "semantic_action_schema_version",
            format!("unsupported semantic action codebook schema version {schema_version}"),
        )),
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SemanticActionCodebook {
    pub(crate) action_kind: ActionKindCodes,
    pub(crate) action_status: ActionStatusCodes,
    pub(crate) action_completeness: ActionCompletenessCodes,
    pub(crate) evidence_kind: EvidenceKindCodes,
    pub(crate) link_role: LinkRoleCodes,
    pub(crate) link_confidence: LinkConfidenceCodes,
}

impl SemanticActionCodebook {
    pub(crate) fn validate(self) -> Result<(), CodebookError> {
        validate_unique("semantic_action_kind", &self.action_kind.entries())?;
        validate_unique("semantic_action_status", &self.action_status.entries())?;
        validate_unique(
            "semantic_action_completeness",
            &self.action_completeness.entries(),
        )?;
        validate_unique("semantic_evidence_kind", &self.evidence_kind.entries())?;
        validate_unique("semantic_action_link_role", &self.link_role.entries())?;
        validate_unique(
            "semantic_action_link_confidence",
            &self.link_confidence.entries(),
        )
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ActionKindCodes {
    pub(crate) process_exec: i16,
    pub(crate) file_modify: i16,
    pub(crate) file_read: i16,
    pub(crate) file_write: i16,
    pub(crate) file_tty_io: i16,
    pub(crate) file_bulk_read: i16,
    pub(crate) fs_enumerate: i16,
    pub(crate) http_message: i16,
    pub(crate) llm_call: i16,
    pub(crate) llm_request: i16,
    pub(crate) llm_response: i16,
    pub(crate) sse_stream: i16,
    pub(crate) sse_event: i16,
    pub(crate) enforcement_decision: i16,
    pub(crate) process_fork_attempt: i16,
    pub(crate) agent_invocation: i16,
    pub(crate) command_invocation: i16,
}

impl ActionKindCodes {
    pub(crate) const fn code(self, value: SemanticActionKind) -> i16 {
        match value {
            SemanticActionKind::ProcessExec => self.process_exec,
            SemanticActionKind::FileModify => self.file_modify,
            SemanticActionKind::FileRead => self.file_read,
            SemanticActionKind::FileWrite => self.file_write,
            SemanticActionKind::FileTtyIo => self.file_tty_io,
            SemanticActionKind::FileBulkRead => self.file_bulk_read,
            SemanticActionKind::FsEnumerate => self.fs_enumerate,
            SemanticActionKind::HttpMessage => self.http_message,
            SemanticActionKind::LlmCall => self.llm_call,
            SemanticActionKind::LlmRequest => self.llm_request,
            SemanticActionKind::LlmResponse => self.llm_response,
            SemanticActionKind::SseStream => self.sse_stream,
            SemanticActionKind::SseEvent => self.sse_event,
            SemanticActionKind::EnforcementDecision => self.enforcement_decision,
            SemanticActionKind::ProcessForkAttempt => self.process_fork_attempt,
            SemanticActionKind::AgentInvocation => self.agent_invocation,
            SemanticActionKind::CommandInvocation => self.command_invocation,
        }
    }

    pub(crate) fn code_from_str(self, value: &str) -> Result<i16, CodebookError> {
        SemanticActionKind::parse(value)
            .map(|kind| self.code(kind))
            .ok_or_else(|| CodebookError::unknown("semantic_action_kind", value))
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticActionKind, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_action_kind_code", code))?;
        match code {
            value if value == self.process_exec => Ok(SemanticActionKind::ProcessExec),
            value if value == self.file_modify => Ok(SemanticActionKind::FileModify),
            value if value == self.file_read => Ok(SemanticActionKind::FileRead),
            value if value == self.file_write => Ok(SemanticActionKind::FileWrite),
            value if value == self.file_tty_io => Ok(SemanticActionKind::FileTtyIo),
            value if value == self.file_bulk_read => Ok(SemanticActionKind::FileBulkRead),
            value if value == self.fs_enumerate => Ok(SemanticActionKind::FsEnumerate),
            value if value == self.http_message => Ok(SemanticActionKind::HttpMessage),
            value if value == self.llm_call => Ok(SemanticActionKind::LlmCall),
            value if value == self.llm_request => Ok(SemanticActionKind::LlmRequest),
            value if value == self.llm_response => Ok(SemanticActionKind::LlmResponse),
            value if value == self.sse_stream => Ok(SemanticActionKind::SseStream),
            value if value == self.sse_event => Ok(SemanticActionKind::SseEvent),
            value if value == self.enforcement_decision => {
                Ok(SemanticActionKind::EnforcementDecision)
            }
            value if value == self.process_fork_attempt => {
                Ok(SemanticActionKind::ProcessForkAttempt)
            }
            value if value == self.agent_invocation => Ok(SemanticActionKind::AgentInvocation),
            value if value == self.command_invocation => Ok(SemanticActionKind::CommandInvocation),
            _ => Err(CodebookError::unknown("semantic_action_kind_code", code)),
        }
    }

    fn entries(self) -> [(&'static str, i16); 17] {
        [
            (SemanticActionKind::ProcessExec.as_str(), self.process_exec),
            (SemanticActionKind::FileModify.as_str(), self.file_modify),
            (SemanticActionKind::FileRead.as_str(), self.file_read),
            (SemanticActionKind::FileWrite.as_str(), self.file_write),
            (SemanticActionKind::FileTtyIo.as_str(), self.file_tty_io),
            (
                SemanticActionKind::FileBulkRead.as_str(),
                self.file_bulk_read,
            ),
            (SemanticActionKind::FsEnumerate.as_str(), self.fs_enumerate),
            (SemanticActionKind::HttpMessage.as_str(), self.http_message),
            (SemanticActionKind::LlmCall.as_str(), self.llm_call),
            (SemanticActionKind::LlmRequest.as_str(), self.llm_request),
            (SemanticActionKind::LlmResponse.as_str(), self.llm_response),
            (SemanticActionKind::SseStream.as_str(), self.sse_stream),
            (SemanticActionKind::SseEvent.as_str(), self.sse_event),
            (
                SemanticActionKind::EnforcementDecision.as_str(),
                self.enforcement_decision,
            ),
            (
                SemanticActionKind::ProcessForkAttempt.as_str(),
                self.process_fork_attempt,
            ),
            (
                SemanticActionKind::AgentInvocation.as_str(),
                self.agent_invocation,
            ),
            (
                SemanticActionKind::CommandInvocation.as_str(),
                self.command_invocation,
            ),
        ]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ActionStatusCodes {
    pub(crate) in_progress: i16,
    pub(crate) success: i16,
    pub(crate) error: i16,
    pub(crate) unknown: i16,
}

impl ActionStatusCodes {
    pub(crate) const fn code(self, value: SemanticActionStatus) -> i16 {
        match value {
            SemanticActionStatus::InProgress => self.in_progress,
            SemanticActionStatus::Success => self.success,
            SemanticActionStatus::Error => self.error,
            SemanticActionStatus::Unknown => self.unknown,
        }
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticActionStatus, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_action_status_code", code))?;
        match code {
            value if value == self.in_progress => Ok(SemanticActionStatus::InProgress),
            value if value == self.success => Ok(SemanticActionStatus::Success),
            value if value == self.error => Ok(SemanticActionStatus::Error),
            value if value == self.unknown => Ok(SemanticActionStatus::Unknown),
            _ => Err(CodebookError::unknown("semantic_action_status_code", code)),
        }
    }

    fn entries(self) -> [(&'static str, i16); 4] {
        [
            (SemanticActionStatus::InProgress.as_str(), self.in_progress),
            (SemanticActionStatus::Success.as_str(), self.success),
            (SemanticActionStatus::Error.as_str(), self.error),
            (SemanticActionStatus::Unknown.as_str(), self.unknown),
        ]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ActionCompletenessCodes {
    pub(crate) complete: i16,
    pub(crate) partial: i16,
    pub(crate) inferred: i16,
}

impl ActionCompletenessCodes {
    pub(crate) const fn code(self, value: SemanticActionCompleteness) -> i16 {
        match value {
            SemanticActionCompleteness::Complete => self.complete,
            SemanticActionCompleteness::Partial => self.partial,
            SemanticActionCompleteness::Inferred => self.inferred,
        }
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticActionCompleteness, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_action_completeness_code", code))?;
        match code {
            value if value == self.complete => Ok(SemanticActionCompleteness::Complete),
            value if value == self.partial => Ok(SemanticActionCompleteness::Partial),
            value if value == self.inferred => Ok(SemanticActionCompleteness::Inferred),
            _ => Err(CodebookError::unknown(
                "semantic_action_completeness_code",
                code,
            )),
        }
    }

    fn entries(self) -> [(&'static str, i16); 3] {
        [
            (SemanticActionCompleteness::Complete.as_str(), self.complete),
            (SemanticActionCompleteness::Partial.as_str(), self.partial),
            (SemanticActionCompleteness::Inferred.as_str(), self.inferred),
        ]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct EvidenceKindCodes {
    pub(crate) event: i16,
    pub(crate) payload_aggregate: i16,
    pub(crate) payload_segment: i16,
}

impl EvidenceKindCodes {
    pub(crate) const fn code(self, value: SemanticEvidenceKind) -> i16 {
        match value {
            SemanticEvidenceKind::Event => self.event,
            SemanticEvidenceKind::PayloadAggregate => self.payload_aggregate,
            SemanticEvidenceKind::PayloadSegment => self.payload_segment,
        }
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticEvidenceKind, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_evidence_kind_code", code))?;
        match code {
            value if value == self.event => Ok(SemanticEvidenceKind::Event),
            value if value == self.payload_aggregate => Ok(SemanticEvidenceKind::PayloadAggregate),
            value if value == self.payload_segment => Ok(SemanticEvidenceKind::PayloadSegment),
            _ => Err(CodebookError::unknown("semantic_evidence_kind_code", code)),
        }
    }

    fn entries(self) -> [(&'static str, i16); 3] {
        [
            (SemanticEvidenceKind::Event.as_str(), self.event),
            (
                SemanticEvidenceKind::PayloadAggregate.as_str(),
                self.payload_aggregate,
            ),
            (
                SemanticEvidenceKind::PayloadSegment.as_str(),
                self.payload_segment,
            ),
        ]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LinkRoleCodes {
    pub(crate) agent_performed_action: i16,
    pub(crate) command_contains_file_access: i16,
    pub(crate) command_contains_process_fork_attempt: i16,
    pub(crate) command_contains_process_exec: i16,
    pub(crate) command_contains_command_invocation: i16,
    pub(crate) command_contains_llm_call: i16,
    pub(crate) file_write_contains_file_event: i16,
    pub(crate) agent_invocation_exec: i16,
    pub(crate) agent_invocation_child_llm_request: i16,
    pub(crate) llm_call_request: i16,
    pub(crate) llm_call_response: i16,
    pub(crate) llm_request_http_message: i16,
    pub(crate) llm_request_llm_response: i16,
    pub(crate) llm_response_http_message: i16,
    pub(crate) llm_response_sse_stream: i16,
    pub(crate) sse_stream_event: i16,
}

impl LinkRoleCodes {
    pub(crate) const fn code(self, value: SemanticActionLinkRole) -> i16 {
        match value {
            SemanticActionLinkRole::AgentPerformedAction => self.agent_performed_action,
            SemanticActionLinkRole::CommandContainsFileAccess => self.command_contains_file_access,
            SemanticActionLinkRole::CommandContainsProcessForkAttempt => {
                self.command_contains_process_fork_attempt
            }
            SemanticActionLinkRole::CommandContainsProcessExec => {
                self.command_contains_process_exec
            }
            SemanticActionLinkRole::CommandContainsCommandInvocation => {
                self.command_contains_command_invocation
            }
            SemanticActionLinkRole::CommandContainsLlmCall => self.command_contains_llm_call,
            SemanticActionLinkRole::FileWriteContainsFileEvent => {
                self.file_write_contains_file_event
            }
            SemanticActionLinkRole::AgentInvocationExec => self.agent_invocation_exec,
            SemanticActionLinkRole::AgentInvocationChildLlmRequest => {
                self.agent_invocation_child_llm_request
            }
            SemanticActionLinkRole::LlmCallRequest => self.llm_call_request,
            SemanticActionLinkRole::LlmCallResponse => self.llm_call_response,
            SemanticActionLinkRole::LlmRequestHttpMessage => self.llm_request_http_message,
            SemanticActionLinkRole::LlmRequestLlmResponse => self.llm_request_llm_response,
            SemanticActionLinkRole::LlmResponseHttpMessage => self.llm_response_http_message,
            SemanticActionLinkRole::LlmResponseSseStream => self.llm_response_sse_stream,
            SemanticActionLinkRole::SseStreamEvent => self.sse_stream_event,
        }
    }

    pub(crate) fn code_from_str(self, value: &str) -> Result<i16, CodebookError> {
        SemanticActionLinkRole::parse(value)
            .map(|role| self.code(role))
            .ok_or_else(|| CodebookError::unknown("semantic_action_link_role", value))
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticActionLinkRole, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_action_link_role_code", code))?;
        match code {
            value if value == self.agent_performed_action => {
                Ok(SemanticActionLinkRole::AgentPerformedAction)
            }
            value if value == self.command_contains_file_access => {
                Ok(SemanticActionLinkRole::CommandContainsFileAccess)
            }
            value if value == self.command_contains_process_fork_attempt => {
                Ok(SemanticActionLinkRole::CommandContainsProcessForkAttempt)
            }
            value if value == self.command_contains_process_exec => {
                Ok(SemanticActionLinkRole::CommandContainsProcessExec)
            }
            value if value == self.command_contains_command_invocation => {
                Ok(SemanticActionLinkRole::CommandContainsCommandInvocation)
            }
            value if value == self.command_contains_llm_call => {
                Ok(SemanticActionLinkRole::CommandContainsLlmCall)
            }
            value if value == self.file_write_contains_file_event => {
                Ok(SemanticActionLinkRole::FileWriteContainsFileEvent)
            }
            value if value == self.agent_invocation_exec => {
                Ok(SemanticActionLinkRole::AgentInvocationExec)
            }
            value if value == self.agent_invocation_child_llm_request => {
                Ok(SemanticActionLinkRole::AgentInvocationChildLlmRequest)
            }
            value if value == self.llm_call_request => Ok(SemanticActionLinkRole::LlmCallRequest),
            value if value == self.llm_call_response => Ok(SemanticActionLinkRole::LlmCallResponse),
            value if value == self.llm_request_http_message => {
                Ok(SemanticActionLinkRole::LlmRequestHttpMessage)
            }
            value if value == self.llm_request_llm_response => {
                Ok(SemanticActionLinkRole::LlmRequestLlmResponse)
            }
            value if value == self.llm_response_http_message => {
                Ok(SemanticActionLinkRole::LlmResponseHttpMessage)
            }
            value if value == self.llm_response_sse_stream => {
                Ok(SemanticActionLinkRole::LlmResponseSseStream)
            }
            value if value == self.sse_stream_event => Ok(SemanticActionLinkRole::SseStreamEvent),
            _ => Err(CodebookError::unknown(
                "semantic_action_link_role_code",
                code,
            )),
        }
    }

    fn entries(self) -> [(&'static str, i16); 16] {
        [
            (
                SemanticActionLinkRole::AgentPerformedAction.as_str(),
                self.agent_performed_action,
            ),
            (
                SemanticActionLinkRole::CommandContainsFileAccess.as_str(),
                self.command_contains_file_access,
            ),
            (
                SemanticActionLinkRole::CommandContainsProcessForkAttempt.as_str(),
                self.command_contains_process_fork_attempt,
            ),
            (
                SemanticActionLinkRole::CommandContainsProcessExec.as_str(),
                self.command_contains_process_exec,
            ),
            (
                SemanticActionLinkRole::CommandContainsCommandInvocation.as_str(),
                self.command_contains_command_invocation,
            ),
            (
                SemanticActionLinkRole::CommandContainsLlmCall.as_str(),
                self.command_contains_llm_call,
            ),
            (
                SemanticActionLinkRole::FileWriteContainsFileEvent.as_str(),
                self.file_write_contains_file_event,
            ),
            (
                SemanticActionLinkRole::AgentInvocationExec.as_str(),
                self.agent_invocation_exec,
            ),
            (
                SemanticActionLinkRole::AgentInvocationChildLlmRequest.as_str(),
                self.agent_invocation_child_llm_request,
            ),
            (
                SemanticActionLinkRole::LlmCallRequest.as_str(),
                self.llm_call_request,
            ),
            (
                SemanticActionLinkRole::LlmCallResponse.as_str(),
                self.llm_call_response,
            ),
            (
                SemanticActionLinkRole::LlmRequestHttpMessage.as_str(),
                self.llm_request_http_message,
            ),
            (
                SemanticActionLinkRole::LlmRequestLlmResponse.as_str(),
                self.llm_request_llm_response,
            ),
            (
                SemanticActionLinkRole::LlmResponseHttpMessage.as_str(),
                self.llm_response_http_message,
            ),
            (
                SemanticActionLinkRole::LlmResponseSseStream.as_str(),
                self.llm_response_sse_stream,
            ),
            (
                SemanticActionLinkRole::SseStreamEvent.as_str(),
                self.sse_stream_event,
            ),
        ]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LinkConfidenceCodes {
    pub(crate) observed: i16,
    pub(crate) derived: i16,
}

impl LinkConfidenceCodes {
    pub(crate) const fn code(self, value: SemanticActionLinkConfidence) -> i16 {
        match value {
            SemanticActionLinkConfidence::Observed => self.observed,
            SemanticActionLinkConfidence::Derived => self.derived,
        }
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticActionLinkConfidence, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_action_link_confidence_code", code))?;
        match code {
            value if value == self.observed => Ok(SemanticActionLinkConfidence::Observed),
            value if value == self.derived => Ok(SemanticActionLinkConfidence::Derived),
            _ => Err(CodebookError::unknown(
                "semantic_action_link_confidence_code",
                code,
            )),
        }
    }

    fn entries(self) -> [(&'static str, i16); 2] {
        [
            (
                SemanticActionLinkConfidence::Observed.as_str(),
                self.observed,
            ),
            (SemanticActionLinkConfidence::Derived.as_str(), self.derived),
        ]
    }
}
