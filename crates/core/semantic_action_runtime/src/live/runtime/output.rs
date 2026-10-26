//! Records, explicit changes and consumer views emitted by live projection.
use model_core::{diagnostics::LlmPipelineDiagnostic, event::DomainEvent, payload::PayloadSegment};
use semantic_action::{
    FileObservationPath, FilePathSetWrite, LlmRequestContentWrite, LlmRequestLineageWrite,
    McpJsonRpcContentWrite, SemanticAction, SemanticActionLink,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LiveMcpStdioMetrics {
    pub untracked_stdio: u64,
    pub candidates: u64,
    pub rejected: u64,
    pub confirmed: u64,
    pub lifecycle_contract_gaps: u64,
    pub capacity_exhausted: u64,
    pub candidate_stream_discards: u64,
    pub confirmed_parse_discards: u64,
    pub rejection_reasons: std::collections::BTreeMap<String, u64>,
    pub discard_reasons: std::collections::BTreeMap<String, u64>,
}

pub struct LiveSemanticActionObservation {
    pub output: LiveSemanticActionOutput,
    pub mcp_stdio_diagnostics: Vec<crate::live::LiveMcpStdioDiagnostic>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSemanticActionOutput {
    pub http1_messages: Vec<crate::Http1MessageEvent>,
    pub http1_handled: bool,
    pub http2_frames: Vec<crate::Http2FrameObservation>,
    pub http2_handled: bool,
    pub payload_consumed_by_http: bool,
    pub payload_consumed_by_llm: bool,
    pub actions: Vec<SemanticAction>,
    pub updates: Vec<semantic_action::SemanticActionUpdate>,
    pub updated_actions: Vec<SemanticAction>,
    pub links: Vec<SemanticActionLink>,
    pub file_observation_paths: Vec<FileObservationPath>,
    pub file_path_sets: Vec<FilePathSetWrite>,
    pub llm_request_contents: Vec<LlmRequestContentWrite>,
    pub llm_request_lineages: Vec<LlmRequestLineageWrite>,
    pub mcp_jsonrpc_contents: Vec<McpJsonRpcContentWrite>,
    pub payload_segments: Vec<PayloadSegment>,
    pub llm_pipeline_diagnostics: Vec<LlmPipelineDiagnostic>,
    pub deferred_events: Vec<DomainEvent>,
    pub retain_event: bool,
    pub raw_event_consumed: bool,
}

impl Default for LiveSemanticActionOutput {
    fn default() -> Self {
        Self {
            http1_messages: Vec::new(),
            http1_handled: false,
            http2_frames: Vec::new(),
            http2_handled: false,
            payload_consumed_by_http: false,
            payload_consumed_by_llm: false,
            actions: Vec::new(),
            updates: Vec::new(),
            updated_actions: Vec::new(),
            links: Vec::new(),
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            llm_request_contents: Vec::new(),
            llm_request_lineages: Vec::new(),
            mcp_jsonrpc_contents: Vec::new(),
            payload_segments: Vec::new(),
            llm_pipeline_diagnostics: Vec::new(),
            deferred_events: Vec::new(),
            retain_event: true,
            raw_event_consumed: false,
        }
    }
}

impl LiveSemanticActionOutput {
    pub(super) fn extend(&mut self, other: Self) {
        self.http1_messages.extend(other.http1_messages);
        self.http1_handled |= other.http1_handled;
        self.http2_frames.extend(other.http2_frames);
        self.http2_handled |= other.http2_handled;
        self.actions.extend(other.actions);
        self.updates.extend(other.updates);
        self.updated_actions.extend(other.updated_actions);
        self.links.extend(other.links);
        self.file_observation_paths
            .extend(other.file_observation_paths);
        self.file_path_sets.extend(other.file_path_sets);
        self.llm_request_contents.extend(other.llm_request_contents);
        self.llm_request_lineages.extend(other.llm_request_lineages);
        self.mcp_jsonrpc_contents.extend(other.mcp_jsonrpc_contents);
        self.payload_segments.extend(other.payload_segments);
        self.llm_pipeline_diagnostics
            .extend(other.llm_pipeline_diagnostics);
        self.deferred_events.extend(other.deferred_events);
        self.retain_event = self.retain_event && other.retain_event;
        self.raw_event_consumed = self.raw_event_consumed || other.raw_event_consumed;
    }
}
