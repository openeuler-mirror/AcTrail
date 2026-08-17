//! Cross-layer semantic action records.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SemanticActionKind {
    ProcessExec,
    ProcessExit,
    AgentIdentity,
    AgentExit,
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
    McpRequest,
    McpResponse,
    McpStdin,
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
            Self::ProcessExit => "process.exit",
            Self::AgentIdentity => "agent.identity",
            Self::AgentExit => "agent.exit",
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
            "process.exit" => Some(Self::ProcessExit),
            "agent.identity" => Some(Self::AgentIdentity),
            "agent.exit" => Some(Self::AgentExit),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
    Unknown,
}

impl FileChangeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "created" => Some(Self::Created),
            "modified" => Some(Self::Modified),
            "deleted" => Some(Self::Deleted),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
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
    pub attributes: BTreeMap<String, String>,
    pub evidence: Vec<SemanticEvidence>,
}

impl SemanticAction {
    pub fn file_change_kind(&self) -> Option<FileChangeKind> {
        self.attributes
            .get(crate::attr_keys::file::CHANGE_KIND)
            .and_then(|raw| FileChangeKind::parse(raw))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionPage {
    pub actions: Vec<SemanticAction>,
    pub next_offset: Option<usize>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LlmTrajectoryTransition {
    Root,
    Append,
    ForkRoot,
    DuplicateRoot,
}

impl LlmTrajectoryTransition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Append => "append",
            Self::ForkRoot => "fork_root",
            Self::DuplicateRoot => "duplicate_root",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "root" => Some(Self::Root),
            "append" => Some(Self::Append),
            "fork_root" => Some(Self::ForkRoot),
            "duplicate_root" => Some(Self::DuplicateRoot),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LlmTrajectoryStartReason {
    Unspecified,
    ContextRewriteOrCompression,
    RuntimeReset,
    CapacityEviction,
    UnsupportedMultimodal,
    HistoryLimit,
    ClassifierFailure,
}

impl LlmTrajectoryStartReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::ContextRewriteOrCompression => "context_rewrite_or_compression",
            Self::RuntimeReset => "runtime_reset",
            Self::CapacityEviction => "capacity_eviction",
            Self::UnsupportedMultimodal => "unsupported_multimodal",
            Self::HistoryLimit => "history_limit",
            Self::ClassifierFailure => "classifier_failure",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unspecified" => Some(Self::Unspecified),
            "context_rewrite_or_compression" => Some(Self::ContextRewriteOrCompression),
            "runtime_reset" => Some(Self::RuntimeReset),
            "capacity_eviction" => Some(Self::CapacityEviction),
            "unsupported_multimodal" => Some(Self::UnsupportedMultimodal),
            "history_limit" => Some(Self::HistoryLimit),
            "classifier_failure" => Some(Self::ClassifierFailure),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRequestLineageWrite {
    pub trace_id: TraceId,
    pub action_id: String,
    pub trajectory_id: String,
    pub parent_action_id: Option<String>,
    pub forked_from_action_id: Option<String>,
    pub trajectory_position: u32,
    pub transition: LlmTrajectoryTransition,
    pub start_reason: LlmTrajectoryStartReason,
    pub inference_version: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmRequestLineage {
    pub trace_id: TraceId,
    pub action_id: String,
    pub trajectory_id: String,
    pub parent_action_id: Option<String>,
    pub forked_from_action_id: Option<String>,
    pub trajectory_position: u32,
    pub transition: LlmTrajectoryTransition,
    pub start_reason: LlmTrajectoryStartReason,
    pub inference_version: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpJsonRpcContentWrite {
    pub trace_id: TraceId,
    pub action_ids: Vec<String>,
    pub format_version: u32,
    pub canonical_json_hash: String,
    pub canonical_json: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpJsonRpcContentPage {
    pub trace_id: TraceId,
    pub action_id: String,
    pub format_version: u32,
    pub canonical_json_hash: String,
    pub canonical_json_bytes: u64,
    pub returned_bytes: u64,
    pub truncated: bool,
    pub canonical_json: String,
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
    McpRequestStdout,
    McpResponseStdin,
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
