//! Semantic action link role names for storage and view queries.

use crate::model::SemanticActionLinkRole;

pub const AGENT_INVOCATION_CHILD_LLM_REQUEST: &str =
    SemanticActionLinkRole::AgentInvocationChildLlmRequest.as_str();
pub const AGENT_INVOCATION_EXEC: &str = SemanticActionLinkRole::AgentInvocationExec.as_str();
pub const AGENT_PERFORMED_ACTION: &str = SemanticActionLinkRole::AgentPerformedAction.as_str();
pub const COMMAND_CONTAINS_COMMAND_INVOCATION: &str =
    SemanticActionLinkRole::CommandContainsCommandInvocation.as_str();
pub const COMMAND_CONTAINS_FILE_ACCESS: &str =
    SemanticActionLinkRole::CommandContainsFileAccess.as_str();
pub const COMMAND_CONTAINS_LLM_CALL: &str = SemanticActionLinkRole::CommandContainsLlmCall.as_str();
pub const COMMAND_CONTAINS_PROCESS_EXEC: &str =
    SemanticActionLinkRole::CommandContainsProcessExec.as_str();
pub const COMMAND_CONTAINS_PROCESS_FORK_ATTEMPT: &str =
    SemanticActionLinkRole::CommandContainsProcessForkAttempt.as_str();
pub const FILE_WRITE_CONTAINS_FILE_EVENT: &str =
    SemanticActionLinkRole::FileWriteContainsFileEvent.as_str();
pub const LLM_CALL_REQUEST: &str = SemanticActionLinkRole::LlmCallRequest.as_str();
pub const LLM_CALL_RESPONSE: &str = SemanticActionLinkRole::LlmCallResponse.as_str();
pub const LLM_REQUEST_HTTP_MESSAGE: &str = SemanticActionLinkRole::LlmRequestHttpMessage.as_str();
pub const LLM_REQUEST_LLM_RESPONSE: &str = SemanticActionLinkRole::LlmRequestLlmResponse.as_str();
pub const LLM_RESPONSE_HTTP_MESSAGE: &str = SemanticActionLinkRole::LlmResponseHttpMessage.as_str();
pub const LLM_RESPONSE_SSE_STREAM: &str = SemanticActionLinkRole::LlmResponseSseStream.as_str();
pub const SSE_STREAM_EVENT: &str = SemanticActionLinkRole::SseStreamEvent.as_str();
