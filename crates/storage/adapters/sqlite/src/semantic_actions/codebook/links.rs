use super::CodebookError;
use semantic_action::SemanticActionLinkRole;

#[derive(Clone, Copy)]
pub(crate) struct LinkRoleCodes {
    pub(crate) llm_request_websocket_message: i16,
    pub(crate) llm_response_websocket_message: i16,
    pub(crate) agent_performed_action: i16,
    pub(crate) command_contains_file_access: i16,
    pub(crate) command_contains_process_fork_attempt: i16,
    pub(crate) command_contains_process_exec: i16,
    pub(crate) command_contains_command_invocation: i16,
    pub(crate) command_contains_llm_call: i16,
    pub(crate) command_contains_mcp_tool_call: i16,
    pub(crate) file_write_contains_file_event: i16,
    pub(crate) agent_invocation_exec: i16,
    pub(crate) agent_invocation_child_llm_request: i16,
    pub(crate) llm_request_trajectory_parent: i16,
    pub(crate) llm_request_trajectory_fork: i16,
    pub(crate) llm_call_request: i16,
    pub(crate) llm_call_response: i16,
    pub(crate) llm_call_http_response: i16,
    pub(crate) llm_request_tool_result: i16,
    pub(crate) llm_response_tool_call: i16,
    pub(crate) llm_tool_call_result: i16,
    pub(crate) llm_tool_call_agent_invocation: i16,
    pub(crate) http_request_http_response: i16,
    pub(crate) llm_request_http_message: i16,
    pub(crate) llm_request_llm_response: i16,
    pub(crate) llm_response_http_message: i16,
    pub(crate) llm_response_sse_stream: i16,
    pub(crate) sse_stream_event: i16,
    pub(crate) mcp_tool_call_request: i16,
    pub(crate) mcp_tool_call_response: i16,
    pub(crate) mcp_request_stdout: i16,
    pub(crate) mcp_response_stdin: i16,
}

impl LinkRoleCodes {
    pub(crate) const fn code(self, value: SemanticActionLinkRole) -> i16 {
        match value {
            SemanticActionLinkRole::LlmResponseWebSocketMessage => {
                self.llm_response_websocket_message
            }
            SemanticActionLinkRole::LlmRequestWebSocketMessage => {
                self.llm_request_websocket_message
            }
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
            SemanticActionLinkRole::CommandContainsMcpToolCall => {
                self.command_contains_mcp_tool_call
            }
            SemanticActionLinkRole::FileWriteContainsFileEvent => {
                self.file_write_contains_file_event
            }
            SemanticActionLinkRole::AgentInvocationExec => self.agent_invocation_exec,
            SemanticActionLinkRole::AgentInvocationChildLlmRequest => {
                self.agent_invocation_child_llm_request
            }
            SemanticActionLinkRole::LlmRequestTrajectoryParent => {
                self.llm_request_trajectory_parent
            }
            SemanticActionLinkRole::LlmRequestTrajectoryFork => self.llm_request_trajectory_fork,
            SemanticActionLinkRole::LlmCallRequest => self.llm_call_request,
            SemanticActionLinkRole::LlmCallResponse => self.llm_call_response,
            SemanticActionLinkRole::LlmCallHttpResponse => self.llm_call_http_response,
            SemanticActionLinkRole::LlmRequestToolResult => self.llm_request_tool_result,
            SemanticActionLinkRole::LlmResponseToolCall => self.llm_response_tool_call,
            SemanticActionLinkRole::LlmToolCallResult => self.llm_tool_call_result,
            SemanticActionLinkRole::LlmToolCallAgentInvocation => {
                self.llm_tool_call_agent_invocation
            }
            SemanticActionLinkRole::HttpRequestHttpResponse => self.http_request_http_response,
            SemanticActionLinkRole::LlmRequestHttpMessage => self.llm_request_http_message,
            SemanticActionLinkRole::LlmRequestLlmResponse => self.llm_request_llm_response,
            SemanticActionLinkRole::LlmResponseHttpMessage => self.llm_response_http_message,
            SemanticActionLinkRole::LlmResponseSseStream => self.llm_response_sse_stream,
            SemanticActionLinkRole::SseStreamEvent => self.sse_stream_event,
            SemanticActionLinkRole::McpToolCallRequest => self.mcp_tool_call_request,
            SemanticActionLinkRole::McpToolCallResponse => self.mcp_tool_call_response,
            SemanticActionLinkRole::McpRequestStdout => self.mcp_request_stdout,
            SemanticActionLinkRole::McpResponseStdin => self.mcp_response_stdin,
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
            value if value == self.llm_response_websocket_message => {
                Ok(SemanticActionLinkRole::LlmResponseWebSocketMessage)
            }
            value if value == self.llm_request_websocket_message => {
                Ok(SemanticActionLinkRole::LlmRequestWebSocketMessage)
            }
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
            value if value == self.command_contains_mcp_tool_call => {
                Ok(SemanticActionLinkRole::CommandContainsMcpToolCall)
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
            value if value == self.llm_request_trajectory_parent => {
                Ok(SemanticActionLinkRole::LlmRequestTrajectoryParent)
            }
            value if value == self.llm_request_trajectory_fork => {
                Ok(SemanticActionLinkRole::LlmRequestTrajectoryFork)
            }
            value if value == self.llm_call_request => Ok(SemanticActionLinkRole::LlmCallRequest),
            value if value == self.llm_call_response => Ok(SemanticActionLinkRole::LlmCallResponse),
            value if value == self.llm_call_http_response => {
                Ok(SemanticActionLinkRole::LlmCallHttpResponse)
            }
            value if value == self.llm_request_tool_result => {
                Ok(SemanticActionLinkRole::LlmRequestToolResult)
            }
            value if value == self.llm_response_tool_call => {
                Ok(SemanticActionLinkRole::LlmResponseToolCall)
            }
            value if value == self.llm_tool_call_result => {
                Ok(SemanticActionLinkRole::LlmToolCallResult)
            }
            value if value == self.llm_tool_call_agent_invocation => {
                Ok(SemanticActionLinkRole::LlmToolCallAgentInvocation)
            }
            value if value == self.http_request_http_response => {
                Ok(SemanticActionLinkRole::HttpRequestHttpResponse)
            }
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
            value if value == self.mcp_tool_call_request => {
                Ok(SemanticActionLinkRole::McpToolCallRequest)
            }
            value if value == self.mcp_tool_call_response => {
                Ok(SemanticActionLinkRole::McpToolCallResponse)
            }
            value if value == self.mcp_request_stdout => {
                Ok(SemanticActionLinkRole::McpRequestStdout)
            }
            value if value == self.mcp_response_stdin => {
                Ok(SemanticActionLinkRole::McpResponseStdin)
            }
            _ => Err(CodebookError::unknown(
                "semantic_action_link_role_code",
                code,
            )),
        }
    }

    pub(super) fn entries(self) -> [(&'static str, i16); 31] {
        [
            (
                SemanticActionLinkRole::LlmResponseWebSocketMessage.as_str(),
                self.llm_response_websocket_message,
            ),
            (
                SemanticActionLinkRole::LlmRequestWebSocketMessage.as_str(),
                self.llm_request_websocket_message,
            ),
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
                SemanticActionLinkRole::CommandContainsMcpToolCall.as_str(),
                self.command_contains_mcp_tool_call,
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
                SemanticActionLinkRole::LlmRequestTrajectoryParent.as_str(),
                self.llm_request_trajectory_parent,
            ),
            (
                SemanticActionLinkRole::LlmRequestTrajectoryFork.as_str(),
                self.llm_request_trajectory_fork,
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
                SemanticActionLinkRole::LlmCallHttpResponse.as_str(),
                self.llm_call_http_response,
            ),
            (
                SemanticActionLinkRole::LlmRequestToolResult.as_str(),
                self.llm_request_tool_result,
            ),
            (
                SemanticActionLinkRole::LlmResponseToolCall.as_str(),
                self.llm_response_tool_call,
            ),
            (
                SemanticActionLinkRole::LlmToolCallResult.as_str(),
                self.llm_tool_call_result,
            ),
            (
                SemanticActionLinkRole::LlmToolCallAgentInvocation.as_str(),
                self.llm_tool_call_agent_invocation,
            ),
            (
                SemanticActionLinkRole::HttpRequestHttpResponse.as_str(),
                self.http_request_http_response,
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
            (
                SemanticActionLinkRole::McpToolCallRequest.as_str(),
                self.mcp_tool_call_request,
            ),
            (
                SemanticActionLinkRole::McpToolCallResponse.as_str(),
                self.mcp_tool_call_response,
            ),
            (
                SemanticActionLinkRole::McpRequestStdout.as_str(),
                self.mcp_request_stdout,
            ),
            (
                SemanticActionLinkRole::McpResponseStdin.as_str(),
                self.mcp_response_stdin,
            ),
        ]
    }
}
