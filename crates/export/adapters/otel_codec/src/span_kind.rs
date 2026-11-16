use opentelemetry_proto::tonic::trace::v1::span::SpanKind;
use semantic_action::SemanticActionKind;

pub(crate) struct SpanKindPolicy;

impl SpanKindPolicy {
    pub(crate) fn classify(kind: SemanticActionKind) -> SpanKind {
        match kind {
            SemanticActionKind::HttpMessage
            | SemanticActionKind::WebSocketMessage
            | SemanticActionKind::LlmCall
            | SemanticActionKind::AgentContextCompact
            | SemanticActionKind::LlmRequest
            | SemanticActionKind::LlmResponse => SpanKind::Client,
            SemanticActionKind::ProcessExec
            | SemanticActionKind::ProcessExit
            | SemanticActionKind::AgentIdentity
            | SemanticActionKind::AgentExit
            | SemanticActionKind::CommandInvocation
            | SemanticActionKind::ProcessForkAttempt
            | SemanticActionKind::AgentInvocation
            | SemanticActionKind::LlmToolCall
            | SemanticActionKind::LlmToolResult
            | SemanticActionKind::FileRead
            | SemanticActionKind::FileWrite
            | SemanticActionKind::FileModify
            | SemanticActionKind::FileTtyIo
            | SemanticActionKind::FileBulkRead
            | SemanticActionKind::FsEnumerate
            | SemanticActionKind::McpToolCall
            | SemanticActionKind::McpRequest
            | SemanticActionKind::McpResponse
            | SemanticActionKind::McpStdin
            | SemanticActionKind::McpStdout
            | SemanticActionKind::SseStream
            | SemanticActionKind::SseEvent
            | SemanticActionKind::EnforcementDecision => SpanKind::Internal,
        }
    }
}
