//! Display-parent roles used by the web action tree.

use semantic_action::SemanticActionLinkRole;

pub(super) const NODE_ID_AGENT: &str = "agent-process";

pub(super) const ROOT_LINK_ROLES: &[SemanticActionLinkRole] =
    &[SemanticActionLinkRole::AgentPerformedAction];
pub(super) const ROOT_LINK_ROLE_STRS: &[&str] = &["agent.performed_action"];

pub(super) const DISPLAY_PARENT_ROLES: &[SemanticActionLinkRole] = &[
    SemanticActionLinkRole::AgentInvocationExec,
    SemanticActionLinkRole::AgentInvocationChildLlmRequest,
    SemanticActionLinkRole::CommandContainsCommandInvocation,
    SemanticActionLinkRole::CommandContainsLlmCall,
    SemanticActionLinkRole::CommandContainsFileAccess,
    SemanticActionLinkRole::CommandContainsProcessForkAttempt,
    SemanticActionLinkRole::CommandContainsProcessExec,
    SemanticActionLinkRole::FileWriteContainsFileEvent,
    SemanticActionLinkRole::LlmCallRequest,
    SemanticActionLinkRole::LlmCallResponse,
    SemanticActionLinkRole::LlmRequestHttpMessage,
    SemanticActionLinkRole::LlmResponseHttpMessage,
    SemanticActionLinkRole::LlmResponseSseStream,
    SemanticActionLinkRole::SseStreamEvent,
    SemanticActionLinkRole::LlmRequestLlmResponse,
];
pub(super) const DISPLAY_PARENT_ROLE_STRS: &[&str] = &[
    "agent.invocation.exec",
    "agent.invocation.child_llm_request",
    "command.contains_command_invocation",
    "command.contains_llm_call",
    "command.contains_file_access",
    "command.contains_process_fork_attempt",
    "command.contains_process_exec",
    "file.write.contains_file_event",
    "llm.call.request",
    "llm.call.response",
    "llm.request.http_message",
    "llm.response.http_message",
    "llm.response.sse_stream",
    "sse.stream.event",
    "llm.request.llm_response",
];
