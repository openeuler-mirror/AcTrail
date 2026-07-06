//! Display-parent roles used by the web action tree.

use semantic_action::SemanticActionLinkRole;

pub(super) const NODE_ID_AGENT: &str = "agent-process";

pub(super) const ROOT_LINK_ROLES: &[SemanticActionLinkRole] =
    &[SemanticActionLinkRole::AgentPerformedAction];

pub(super) const DISPLAY_PARENT_ROLES: &[SemanticActionLinkRole] = &[
    SemanticActionLinkRole::AgentInvocationExec,
    SemanticActionLinkRole::AgentInvocationChildLlmRequest,
    SemanticActionLinkRole::CommandContainsCommandInvocation,
    SemanticActionLinkRole::CommandContainsLlmCall,
    SemanticActionLinkRole::CommandContainsMcpToolCall,
    SemanticActionLinkRole::CommandContainsFileAccess,
    SemanticActionLinkRole::CommandContainsProcessForkAttempt,
    SemanticActionLinkRole::CommandContainsProcessExec,
    SemanticActionLinkRole::FileWriteContainsFileEvent,
    SemanticActionLinkRole::LlmCallRequest,
    SemanticActionLinkRole::LlmCallResponse,
    SemanticActionLinkRole::LlmRequestHttpMessage,
    SemanticActionLinkRole::LlmResponseHttpMessage,
    SemanticActionLinkRole::LlmResponseSseStream,
    SemanticActionLinkRole::McpToolCallRequest,
    SemanticActionLinkRole::McpToolCallResponse,
    SemanticActionLinkRole::McpRequestStdout,
    SemanticActionLinkRole::McpResponseStdin,
    SemanticActionLinkRole::McpRequestClientSend,
    SemanticActionLinkRole::McpResponseClientReceive,
    SemanticActionLinkRole::SseStreamEvent,
    SemanticActionLinkRole::LlmRequestLlmResponse,
];
