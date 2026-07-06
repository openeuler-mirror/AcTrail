//! Cross-layer semantic action records.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionKind {
    ProcessExec,
    FileModify,
    FileRead,
    FileWrite,
    FileTtyIo,
    FileBulkRead,
    FsEnumerate,
    HttpMessage,
    LlmCall,
    LlmRequest,
    LlmResponse,
    McpToolCall,
    /// MCP JSON-RPC request role. For stdio transport this is the AI
    /// agent/client process stdout and the MCP server process stdin.
    McpRequest,
    /// MCP JSON-RPC response role. For stdio transport this is the AI
    /// agent/client process stdin and the MCP server process stdout.
    McpResponse,
    /// Client-to-server remote MCP transport payload.
    McpClientSend,
    /// Server-to-client remote MCP transport payload.
    McpClientReceive,
    /// Stdio bytes read by the AI agent/client process from the MCP server.
    McpStdin,
    /// Stdio bytes written by the AI agent/client process to the MCP server.
    McpStdout,
    SseStream,
    SseEvent,
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
            Self::FileRead => "file.read",
            Self::FileWrite => "file.write",
            Self::FileTtyIo => "file.tty_io",
            Self::FileBulkRead => "file.bulk_read",
            Self::FsEnumerate => "fs.enumerate",
            Self::HttpMessage => "http.message",
            Self::LlmCall => "llm.call",
            Self::LlmRequest => "llm.request",
            Self::LlmResponse => "llm.response",
            Self::McpToolCall => "mcp.tool_call",
            Self::McpRequest => "mcp.request",
            Self::McpResponse => "mcp.response",
            Self::McpClientSend => "mcp.client_send",
            Self::McpClientReceive => "mcp.client_receive",
            Self::McpStdin => "mcp.stdin",
            Self::McpStdout => "mcp.stdout",
            Self::SseStream => "sse.stream",
            Self::SseEvent => "sse.event",
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
            "file.read" => Some(Self::FileRead),
            "file.write" => Some(Self::FileWrite),
            "file.tty_io" => Some(Self::FileTtyIo),
            "file.bulk_read" => Some(Self::FileBulkRead),
            "fs.enumerate" => Some(Self::FsEnumerate),
            "http.message" => Some(Self::HttpMessage),
            "llm.call" => Some(Self::LlmCall),
            "llm.request" => Some(Self::LlmRequest),
            "llm.response" => Some(Self::LlmResponse),
            "mcp.tool_call" => Some(Self::McpToolCall),
            "mcp.request" => Some(Self::McpRequest),
            "mcp.response" => Some(Self::McpResponse),
            "mcp.client_send" => Some(Self::McpClientSend),
            "mcp.client_receive" => Some(Self::McpClientReceive),
            "mcp.stdin" => Some(Self::McpStdin),
            "mcp.stdout" => Some(Self::McpStdout),
            "sse.stream" => Some(Self::SseStream),
            "sse.event" => Some(Self::SseEvent),
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
    PayloadAggregate,
    PayloadSegment,
}

impl SemanticEvidenceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::PayloadAggregate => "payload_aggregate",
            Self::PayloadSegment => "payload_segment",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "event" => Some(Self::Event),
            "payload_aggregate" => Some(Self::PayloadAggregate),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObservationPath {
    pub trace_id: TraceId,
    pub action_id: String,
    pub path_order: u32,
    pub path: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilePathSetState {
    Pending,
    Complete,
    Overflow,
}

impl FilePathSetState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Complete => "complete",
            Self::Overflow => "overflow",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "complete" => Some(Self::Complete),
            "overflow" => Some(Self::Overflow),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePathSetWrite {
    pub trace_id: TraceId,
    pub action_id: String,
    pub path_set_id: String,
    pub state: FilePathSetState,
    pub unique_path_count: u64,
    pub stored_path_count: u64,
    pub chunking_scheme: String,
    pub chunk_max_paths: u32,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePathSetIdentity {
    pub path_set_id: String,
    pub path_set_hash: String,
}

pub fn file_path_set_identity_for_paths<I, P>(
    state: FilePathSetState,
    chunking_scheme: &str,
    paths: I,
) -> FilePathSetIdentity
where
    I: IntoIterator<Item = P>,
    P: AsRef<str>,
{
    let sorted_paths = paths
        .into_iter()
        .map(|path| path.as_ref().to_string())
        .collect::<BTreeSet<_>>();
    let mut input = format!(
        "file-path-set-v1\nstate:{}\nchunking:{}:",
        state.as_str(),
        chunking_scheme.len()
    );
    input.push_str(chunking_scheme);
    input.push_str("\ncount:");
    input.push_str(&sorted_paths.len().to_string());
    input.push('\n');
    for path in sorted_paths {
        input.push_str(&path.len().to_string());
        input.push(':');
        input.push_str(&path);
        input.push('\n');
    }
    let path_set_hash = stable_hash_bytes(input.as_bytes());
    FilePathSetIdentity {
        path_set_id: format!("file-path-set:{path_set_hash}"),
        path_set_hash,
    }
}

pub fn file_path_set_identity_for_overflow_scope(
    chunking_scheme: &str,
    scope: &str,
) -> FilePathSetIdentity {
    let mut input = format!(
        "file-path-set-overflow-scope-v1\nchunking:{}:",
        chunking_scheme.len()
    );
    input.push_str(chunking_scheme);
    input.push_str("\nscope:");
    input.push_str(&scope.len().to_string());
    input.push(':');
    input.push_str(scope);
    input.push('\n');
    let path_set_hash = stable_hash_bytes(input.as_bytes());
    FilePathSetIdentity {
        path_set_id: format!("file-path-set:{path_set_hash}"),
        path_set_hash,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePathSetPath {
    pub path_id: u64,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePathSetPathPage {
    pub path_set_id: String,
    pub action_id: String,
    pub state: FilePathSetState,
    pub unique_path_count: u64,
    pub stored_path_count: u64,
    pub chunking_scheme: String,
    pub paths: Vec<FilePathSetPath>,
    pub total_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRequestManifest {
    pub trace_id: TraceId,
    pub action_id: String,
    pub format_version: u32,
    pub canonical_body_hash: String,
    pub canonical_body_bytes: u64,
    pub skeleton_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRequestBlockRef {
    pub trace_id: TraceId,
    pub action_id: String,
    pub ordinal: u32,
    pub block_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRequestBlock {
    pub trace_id: TraceId,
    pub block_hash: String,
    pub uncompressed_bytes: u64,
    pub encoded_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRequestContentWrite {
    pub manifest: LlmRequestManifest,
    pub block_refs: Vec<LlmRequestBlockRef>,
    pub blocks: Vec<LlmRequestBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRequestContentPage {
    pub trace_id: TraceId,
    pub action_id: String,
    pub format_version: u32,
    pub canonical_body_hash: String,
    pub canonical_body_bytes: u64,
    pub returned_bytes: u64,
    pub truncated: bool,
    pub body_json: String,
}

const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
const FNV_PRIME: u64 = 1_099_511_628_211;

fn stable_hash_bytes(bytes: &[u8]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SemanticActionLinkRole {
    AgentPerformedAction,
    CommandContainsFileAccess,
    CommandContainsProcessForkAttempt,
    CommandContainsProcessExec,
    CommandContainsCommandInvocation,
    CommandContainsLlmCall,
    CommandContainsMcpToolCall,
    FileWriteContainsFileEvent,
    AgentInvocationExec,
    AgentInvocationChildLlmRequest,
    LlmCallRequest,
    LlmCallResponse,
    LlmRequestHttpMessage,
    LlmRequestLlmResponse,
    LlmResponseHttpMessage,
    LlmResponseSseStream,
    SseStreamEvent,
    McpToolCallRequest,
    McpToolCallResponse,
    /// Links a protocol-level MCP request to the client-side stdout bytes that
    /// carried it into the MCP server stdin.
    McpRequestStdout,
    /// Links a protocol-level MCP response to the client-side stdin bytes read
    /// from the MCP server stdout.
    McpResponseStdin,
    /// Links a protocol-level remote MCP request to the client-side HTTP
    /// request payload sent to the MCP server.
    McpRequestClientSend,
    /// Links a protocol-level remote MCP response to the client-side HTTP
    /// response payload received from the MCP server.
    McpResponseClientReceive,
}

impl SemanticActionLinkRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentPerformedAction => "agent.performed_action",
            Self::CommandContainsFileAccess => "command.contains_file_access",
            Self::CommandContainsProcessForkAttempt => "command.contains_process_fork_attempt",
            Self::CommandContainsProcessExec => "command.contains_process_exec",
            Self::CommandContainsCommandInvocation => "command.contains_command_invocation",
            Self::CommandContainsLlmCall => "command.contains_llm_call",
            Self::CommandContainsMcpToolCall => "command.contains_mcp_tool_call",
            Self::FileWriteContainsFileEvent => "file.write.contains_file_event",
            Self::AgentInvocationExec => "agent.invocation.exec",
            Self::AgentInvocationChildLlmRequest => "agent.invocation.child_llm_request",
            Self::LlmCallRequest => "llm.call.request",
            Self::LlmCallResponse => "llm.call.response",
            Self::LlmRequestHttpMessage => "llm.request.http_message",
            Self::LlmRequestLlmResponse => "llm.request.llm_response",
            Self::LlmResponseHttpMessage => "llm.response.http_message",
            Self::LlmResponseSseStream => "llm.response.sse_stream",
            Self::SseStreamEvent => "sse.stream.event",
            Self::McpToolCallRequest => "mcp.tool_call.request",
            Self::McpToolCallResponse => "mcp.tool_call.response",
            Self::McpRequestStdout => "mcp.request.stdout",
            Self::McpResponseStdin => "mcp.response.stdin",
            Self::McpRequestClientSend => "mcp.request.client_send",
            Self::McpResponseClientReceive => "mcp.response.client_receive",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "agent.performed_action" => Some(Self::AgentPerformedAction),
            "command.contains_file_access" => Some(Self::CommandContainsFileAccess),
            "command.contains_process_fork_attempt" => {
                Some(Self::CommandContainsProcessForkAttempt)
            }
            "command.contains_process_exec" => Some(Self::CommandContainsProcessExec),
            "command.contains_command_invocation" => Some(Self::CommandContainsCommandInvocation),
            "command.contains_llm_call" => Some(Self::CommandContainsLlmCall),
            "command.contains_mcp_tool_call" => Some(Self::CommandContainsMcpToolCall),
            "file.write.contains_file_event" => Some(Self::FileWriteContainsFileEvent),
            "agent.invocation.exec" => Some(Self::AgentInvocationExec),
            "agent.invocation.child_llm_request" => Some(Self::AgentInvocationChildLlmRequest),
            "llm.call.request" => Some(Self::LlmCallRequest),
            "llm.call.response" => Some(Self::LlmCallResponse),
            "llm.request.http_message" => Some(Self::LlmRequestHttpMessage),
            "llm.request.llm_response" => Some(Self::LlmRequestLlmResponse),
            "llm.response.http_message" => Some(Self::LlmResponseHttpMessage),
            "llm.response.sse_stream" => Some(Self::LlmResponseSseStream),
            "sse.stream.event" => Some(Self::SseStreamEvent),
            "mcp.tool_call.request" => Some(Self::McpToolCallRequest),
            "mcp.tool_call.response" => Some(Self::McpToolCallResponse),
            "mcp.request.stdout" => Some(Self::McpRequestStdout),
            "mcp.response.stdin" => Some(Self::McpResponseStdin),
            "mcp.request.client_send" => Some(Self::McpRequestClientSend),
            "mcp.response.client_receive" => Some(Self::McpResponseClientReceive),
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
    pub valid: bool,
    pub evidence: Vec<SemanticEvidence>,
    pub attributes: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::{SemanticActionKind, SemanticActionLinkRole};
    use crate::{attr_keys, evidence_roles, link_roles};

    #[test]
    fn mcp_remote_client_message_kinds_round_trip() {
        assert_eq!(
            SemanticActionKind::McpClientSend.as_str(),
            "mcp.client_send"
        );
        assert_eq!(
            SemanticActionKind::parse("mcp.client_send"),
            Some(SemanticActionKind::McpClientSend)
        );
        assert_eq!(
            SemanticActionKind::McpClientReceive.as_str(),
            "mcp.client_receive"
        );
        assert_eq!(
            SemanticActionKind::parse("mcp.client_receive"),
            Some(SemanticActionKind::McpClientReceive)
        );
    }

    #[test]
    fn command_contains_mcp_tool_call_role_round_trips() {
        assert_eq!(
            SemanticActionLinkRole::CommandContainsMcpToolCall.as_str(),
            "command.contains_mcp_tool_call"
        );
        assert_eq!(
            SemanticActionLinkRole::parse("command.contains_mcp_tool_call"),
            Some(SemanticActionLinkRole::CommandContainsMcpToolCall)
        );
        assert_eq!(
            link_roles::COMMAND_CONTAINS_MCP_TOOL_CALL,
            "command.contains_mcp_tool_call"
        );
    }

    #[test]
    fn mcp_tool_call_child_roles_round_trip() {
        assert_eq!(
            SemanticActionLinkRole::McpToolCallRequest.as_str(),
            "mcp.tool_call.request"
        );
        assert_eq!(
            SemanticActionLinkRole::parse("mcp.tool_call.request"),
            Some(SemanticActionLinkRole::McpToolCallRequest)
        );
        assert_eq!(
            SemanticActionLinkRole::McpToolCallResponse.as_str(),
            "mcp.tool_call.response"
        );
        assert_eq!(
            SemanticActionLinkRole::parse("mcp.tool_call.response"),
            Some(SemanticActionLinkRole::McpToolCallResponse)
        );
        assert_eq!(
            SemanticActionLinkRole::McpRequestStdout.as_str(),
            "mcp.request.stdout"
        );
        assert_eq!(
            SemanticActionLinkRole::parse("mcp.request.stdout"),
            Some(SemanticActionLinkRole::McpRequestStdout)
        );
        assert_eq!(
            SemanticActionLinkRole::McpResponseStdin.as_str(),
            "mcp.response.stdin"
        );
        assert_eq!(
            SemanticActionLinkRole::parse("mcp.response.stdin"),
            Some(SemanticActionLinkRole::McpResponseStdin)
        );
        assert_eq!(
            SemanticActionLinkRole::McpRequestClientSend.as_str(),
            "mcp.request.client_send"
        );
        assert_eq!(
            SemanticActionLinkRole::parse("mcp.request.client_send"),
            Some(SemanticActionLinkRole::McpRequestClientSend)
        );
        assert_eq!(
            SemanticActionLinkRole::McpResponseClientReceive.as_str(),
            "mcp.response.client_receive"
        );
        assert_eq!(
            SemanticActionLinkRole::parse("mcp.response.client_receive"),
            Some(SemanticActionLinkRole::McpResponseClientReceive)
        );
        assert_eq!(
            link_roles::MCP_REQUEST_CLIENT_SEND,
            "mcp.request.client_send"
        );
        assert_eq!(
            link_roles::MCP_RESPONSE_CLIENT_RECEIVE,
            "mcp.response.client_receive"
        );
    }

    #[test]
    fn mcp_remote_client_payload_constants_are_public() {
        assert_eq!(
            attr_keys::mcp::CLIENT_SEND_ACTION_ID,
            "mcp.client_send.action_id"
        );
        assert_eq!(
            attr_keys::mcp::CLIENT_RECEIVE_ACTION_ID,
            "mcp.client_receive.action_id"
        );
        assert_eq!(
            evidence_roles::mcp::CLIENT_SEND_PAYLOAD,
            "mcp.client_send.payload"
        );
        assert_eq!(
            evidence_roles::mcp::CLIENT_RECEIVE_PAYLOAD,
            "mcp.client_receive.payload"
        );
    }
}
