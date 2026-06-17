//! Display-parent roles used by the web action tree.

use semantic_action::{SemanticActionLinkRole, link_roles};

pub(super) const NODE_ID_AGENT: &str = "agent-process";

pub(super) const ROOT_LINK_ROLES: &[SemanticActionLinkRole] =
    &[SemanticActionLinkRole::AgentPerformedAction];
pub(super) const ROOT_LINK_ROLE_STRS: &[&str] = &[link_roles::AGENT_PERFORMED_ACTION];

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
    link_roles::AGENT_INVOCATION_EXEC,
    link_roles::AGENT_INVOCATION_CHILD_LLM_REQUEST,
    link_roles::COMMAND_CONTAINS_COMMAND_INVOCATION,
    link_roles::COMMAND_CONTAINS_LLM_CALL,
    link_roles::COMMAND_CONTAINS_FILE_ACCESS,
    link_roles::COMMAND_CONTAINS_PROCESS_FORK_ATTEMPT,
    link_roles::COMMAND_CONTAINS_PROCESS_EXEC,
    link_roles::FILE_WRITE_CONTAINS_FILE_EVENT,
    link_roles::LLM_CALL_REQUEST,
    link_roles::LLM_CALL_RESPONSE,
    link_roles::LLM_REQUEST_HTTP_MESSAGE,
    link_roles::LLM_RESPONSE_HTTP_MESSAGE,
    link_roles::LLM_RESPONSE_SSE_STREAM,
    link_roles::SSE_STREAM_EVENT,
    link_roles::LLM_REQUEST_LLM_RESPONSE,
];
