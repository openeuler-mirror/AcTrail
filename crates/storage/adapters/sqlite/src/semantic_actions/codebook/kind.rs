use super::CodebookError;
use semantic_action::SemanticActionKind;

#[derive(Clone, Copy)]
pub(crate) struct ActionKindCodes {
    pub(crate) process_exec: i16,
    pub(crate) process_exit: i16,
    pub(crate) agent_identity: i16,
    pub(crate) agent_exit: i16,
    pub(crate) file_modify: i16,
    pub(crate) file_read: i16,
    pub(crate) file_write: i16,
    pub(crate) file_tty_io: i16,
    pub(crate) file_bulk_read: i16,
    pub(crate) fs_enumerate: i16,
    pub(crate) http_message: i16,
    pub(crate) websocket_message: i16,
    pub(crate) llm_call: i16,
    pub(crate) agent_context_compact: i16,
    pub(crate) llm_request: i16,
    pub(crate) llm_response: i16,
    pub(crate) llm_tool_call: i16,
    pub(crate) llm_tool_result: i16,
    pub(crate) mcp_tool_call: i16,
    pub(crate) mcp_request: i16,
    pub(crate) mcp_response: i16,
    pub(crate) mcp_stdin: i16,
    pub(crate) mcp_stdout: i16,
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
            SemanticActionKind::ProcessExit => self.process_exit,
            SemanticActionKind::AgentIdentity => self.agent_identity,
            SemanticActionKind::AgentExit => self.agent_exit,
            SemanticActionKind::FileModify => self.file_modify,
            SemanticActionKind::FileRead => self.file_read,
            SemanticActionKind::FileWrite => self.file_write,
            SemanticActionKind::FileTtyIo => self.file_tty_io,
            SemanticActionKind::FileBulkRead => self.file_bulk_read,
            SemanticActionKind::FsEnumerate => self.fs_enumerate,
            SemanticActionKind::HttpMessage => self.http_message,
            SemanticActionKind::WebSocketMessage => self.websocket_message,
            SemanticActionKind::LlmCall => self.llm_call,
            SemanticActionKind::AgentContextCompact => self.agent_context_compact,
            SemanticActionKind::LlmRequest => self.llm_request,
            SemanticActionKind::LlmResponse => self.llm_response,
            SemanticActionKind::LlmToolCall => self.llm_tool_call,
            SemanticActionKind::LlmToolResult => self.llm_tool_result,
            SemanticActionKind::McpToolCall => self.mcp_tool_call,
            SemanticActionKind::McpRequest => self.mcp_request,
            SemanticActionKind::McpResponse => self.mcp_response,
            SemanticActionKind::McpStdin => self.mcp_stdin,
            SemanticActionKind::McpStdout => self.mcp_stdout,
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
            value if value == self.process_exit => Ok(SemanticActionKind::ProcessExit),
            value if value == self.agent_identity => Ok(SemanticActionKind::AgentIdentity),
            value if value == self.agent_exit => Ok(SemanticActionKind::AgentExit),
            value if value == self.file_modify => Ok(SemanticActionKind::FileModify),
            value if value == self.file_read => Ok(SemanticActionKind::FileRead),
            value if value == self.file_write => Ok(SemanticActionKind::FileWrite),
            value if value == self.file_tty_io => Ok(SemanticActionKind::FileTtyIo),
            value if value == self.file_bulk_read => Ok(SemanticActionKind::FileBulkRead),
            value if value == self.fs_enumerate => Ok(SemanticActionKind::FsEnumerate),
            value if value == self.http_message => Ok(SemanticActionKind::HttpMessage),
            value if value == self.websocket_message => Ok(SemanticActionKind::WebSocketMessage),
            value if value == self.llm_call => Ok(SemanticActionKind::LlmCall),
            value if value == self.agent_context_compact => {
                Ok(SemanticActionKind::AgentContextCompact)
            }
            value if value == self.llm_request => Ok(SemanticActionKind::LlmRequest),
            value if value == self.llm_response => Ok(SemanticActionKind::LlmResponse),
            value if value == self.llm_tool_call => Ok(SemanticActionKind::LlmToolCall),
            value if value == self.llm_tool_result => Ok(SemanticActionKind::LlmToolResult),
            value if value == self.mcp_tool_call => Ok(SemanticActionKind::McpToolCall),
            value if value == self.mcp_request => Ok(SemanticActionKind::McpRequest),
            value if value == self.mcp_response => Ok(SemanticActionKind::McpResponse),
            value if value == self.mcp_stdin => Ok(SemanticActionKind::McpStdin),
            value if value == self.mcp_stdout => Ok(SemanticActionKind::McpStdout),
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

    pub(super) fn entries(self) -> [(&'static str, i16); 29] {
        [
            (SemanticActionKind::ProcessExec.as_str(), self.process_exec),
            (SemanticActionKind::ProcessExit.as_str(), self.process_exit),
            (
                SemanticActionKind::AgentIdentity.as_str(),
                self.agent_identity,
            ),
            (SemanticActionKind::AgentExit.as_str(), self.agent_exit),
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
            (
                SemanticActionKind::WebSocketMessage.as_str(),
                self.websocket_message,
            ),
            (SemanticActionKind::LlmCall.as_str(), self.llm_call),
            (
                SemanticActionKind::AgentContextCompact.as_str(),
                self.agent_context_compact,
            ),
            (SemanticActionKind::LlmRequest.as_str(), self.llm_request),
            (SemanticActionKind::LlmResponse.as_str(), self.llm_response),
            (SemanticActionKind::LlmToolCall.as_str(), self.llm_tool_call),
            (
                SemanticActionKind::LlmToolResult.as_str(),
                self.llm_tool_result,
            ),
            (SemanticActionKind::McpToolCall.as_str(), self.mcp_tool_call),
            (SemanticActionKind::McpRequest.as_str(), self.mcp_request),
            (SemanticActionKind::McpResponse.as_str(), self.mcp_response),
            (SemanticActionKind::McpStdin.as_str(), self.mcp_stdin),
            (SemanticActionKind::McpStdout.as_str(), self.mcp_stdout),
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
