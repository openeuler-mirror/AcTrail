use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

use super::model::McpStdioStream;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::live) struct McpStdioMetrics {
    pub(in crate::live) untracked_stdio: u64,
    pub(in crate::live) candidates: u64,
    pub(in crate::live) rejected: u64,
    pub(in crate::live) confirmed: u64,
    pub(in crate::live) lifecycle_contract_gaps: u64,
    pub(in crate::live) capacity_exhausted: u64,
    pub(in crate::live) candidate_stream_discards: u64,
    pub(in crate::live) confirmed_parse_discards: u64,
    pub(in crate::live) rejection_reasons: BTreeMap<&'static str, u64>,
    pub(in crate::live) discard_reasons: BTreeMap<&'static str, u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpStdioDiagnosticKind {
    LifecycleContractGap,
    CapacityExhausted,
    CandidateRejected,
    CandidateStreamDiscarded,
    ConfirmedStreamDiscarded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveMcpStdioDiagnostic {
    trace_id: TraceId,
    process: ProcessIdentity,
    emitted_at: SystemTime,
    kind: McpStdioDiagnosticKind,
    reason: String,
    stream: Option<McpStdioStream>,
}

impl LiveMcpStdioDiagnostic {
    pub(super) fn lifecycle_contract_gap(
        trace_id: TraceId,
        process: &ProcessIdentity,
        emitted_at: SystemTime,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(
            trace_id,
            process,
            emitted_at,
            McpStdioDiagnosticKind::LifecycleContractGap,
            reason,
            None,
        )
    }

    pub(super) fn capacity_exhausted(
        trace_id: TraceId,
        process: &ProcessIdentity,
        emitted_at: SystemTime,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(
            trace_id,
            process,
            emitted_at,
            McpStdioDiagnosticKind::CapacityExhausted,
            reason,
            None,
        )
    }

    pub(super) fn candidate_rejected(
        trace_id: TraceId,
        process: &ProcessIdentity,
        emitted_at: SystemTime,
        reason: impl Into<String>,
        stream: Option<McpStdioStream>,
    ) -> Self {
        Self::new(
            trace_id,
            process,
            emitted_at,
            McpStdioDiagnosticKind::CandidateRejected,
            reason,
            stream,
        )
    }

    pub(super) fn candidate_stream_discarded(
        trace_id: TraceId,
        process: &ProcessIdentity,
        emitted_at: SystemTime,
        reason: impl Into<String>,
        stream: McpStdioStream,
    ) -> Self {
        Self::new(
            trace_id,
            process,
            emitted_at,
            McpStdioDiagnosticKind::CandidateStreamDiscarded,
            reason,
            Some(stream),
        )
    }

    pub(super) fn confirmed_stream_discarded(
        trace_id: TraceId,
        process: &ProcessIdentity,
        emitted_at: SystemTime,
        reason: impl Into<String>,
        stream: McpStdioStream,
    ) -> Self {
        Self::new(
            trace_id,
            process,
            emitted_at,
            McpStdioDiagnosticKind::ConfirmedStreamDiscarded,
            reason,
            Some(stream),
        )
    }

    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    pub fn process(&self) -> &ProcessIdentity {
        &self.process
    }

    pub fn emitted_at(&self) -> SystemTime {
        self.emitted_at
    }

    pub fn code(&self) -> &'static str {
        match self.kind {
            McpStdioDiagnosticKind::LifecycleContractGap => "mcp_stdio_lifecycle_contract_gap",
            McpStdioDiagnosticKind::CapacityExhausted => "mcp_stdio_capacity_exhausted",
            McpStdioDiagnosticKind::CandidateRejected => "mcp_stdio_candidate_rejected",
            McpStdioDiagnosticKind::CandidateStreamDiscarded => {
                "mcp_stdio_candidate_stream_discarded"
            }
            McpStdioDiagnosticKind::ConfirmedStreamDiscarded => {
                "mcp_stdio_confirmed_stream_discarded"
            }
        }
    }

    pub fn stage(&self) -> &'static str {
        match self.kind {
            McpStdioDiagnosticKind::LifecycleContractGap
            | McpStdioDiagnosticKind::CapacityExhausted => "lifecycle",
            McpStdioDiagnosticKind::CandidateRejected
            | McpStdioDiagnosticKind::CandidateStreamDiscarded => "candidate",
            McpStdioDiagnosticKind::ConfirmedStreamDiscarded => "confirmed",
        }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn stream(&self) -> Option<&'static str> {
        self.stream.map(McpStdioStream::as_str)
    }

    pub fn recoverable(&self) -> bool {
        matches!(
            self.kind,
            McpStdioDiagnosticKind::CandidateStreamDiscarded
                | McpStdioDiagnosticKind::ConfirmedStreamDiscarded
        )
    }

    pub fn message(&self) -> String {
        let stream = self
            .stream()
            .map(|stream| format!(" stream={stream}"))
            .unwrap_or_default();
        format!(
            "MCP stdio observation {}: reason={}{}",
            self.code(),
            self.reason,
            stream
        )
    }

    fn new(
        trace_id: TraceId,
        process: &ProcessIdentity,
        emitted_at: SystemTime,
        kind: McpStdioDiagnosticKind,
        reason: impl Into<String>,
        stream: Option<McpStdioStream>,
    ) -> Self {
        Self {
            trace_id,
            process: process.clone(),
            emitted_at,
            kind,
            reason: reason.into(),
            stream,
        }
    }
}
