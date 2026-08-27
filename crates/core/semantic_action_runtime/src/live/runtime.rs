//! Live semantic action runtime.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{
    AgentInvocationConfig, FileObservationConfig, PayloadMcpConfig, SemanticRetentionConfig,
};
use model_core::diagnostics::LlmPipelineDiagnostic;
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use semantic_action::{
    FileObservationPath, FilePathSetWrite, LlmRequestContentWrite, LlmRequestLineageWrite,
    McpJsonRpcContentWrite, SemanticAction, SemanticActionKind, SemanticActionLink,
    SemanticEvidenceKind, attr_keys as attrs,
};

use crate::llm_pipeline::{
    ActionBatch, LlmActionPipeline, LlmCodecPlugin, LlmCodecPluginStatus, PipelineEvent,
};

use super::actions::{
    enforcement_action, file_modify_action, http_message_action, is_file_modify_event,
    is_http_protocol, process_fork_attempt_action,
};
use super::agent::AgentProjector;
use super::command::CommandProjector;
use super::file::FileAccessProjector;
use super::http_exchange::{DamagedHttp1RequestOutcome, HttpExchangeTracker};
use super::links::ActionLinkProjector;
use super::mcp::{LiveMcpProjector, LiveMcpStdioDiagnostic};
use super::tool::{ToolInteractionProjector, ToolProjectionBatch};

pub struct LiveSemanticActionRuntime {
    agent: AgentProjector,
    command: CommandProjector,
    file_access: FileAccessProjector,
    http_exchange: HttpExchangeTracker,
    llm: LlmActionPipeline,
    mcp: LiveMcpProjector,
    tool: ToolInteractionProjector,
    links: ActionLinkProjector,
}

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
    pub rejection_reasons: BTreeMap<String, u64>,
    pub discard_reasons: BTreeMap<String, u64>,
}

pub struct LiveSemanticActionObservation {
    pub output: LiveSemanticActionOutput,
    pub mcp_stdio_diagnostics: Vec<LiveMcpStdioDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSemanticActionOutput {
    pub actions: Vec<SemanticAction>,
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
            actions: Vec::new(),
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
    fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
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

impl LiveSemanticActionRuntime {
    pub fn new(
        config: AgentInvocationConfig,
        pending_exec_max_entries: u32,
        semantic_retention: SemanticRetentionConfig,
        file_observation: FileObservationConfig,
        mcp: PayloadMcpConfig,
    ) -> Self {
        let enabled = config.enabled;
        let max_tool_entries_per_trace = semantic_retention
            .l0_llm_call
            .projection_state
            .max_tool_entries_per_trace;
        let mcp_content_retention = semantic_retention.l0_mcp_call.clone();
        let http_exchange_config = semantic_retention.l2_http.exchange;
        Self {
            agent: AgentProjector::new(enabled, pending_exec_max_entries),
            command: CommandProjector::new(),
            file_access: FileAccessProjector::new(file_observation),
            http_exchange: HttpExchangeTracker::new(http_exchange_config),
            llm: LlmActionPipeline::new(semantic_retention),
            mcp: LiveMcpProjector::new(mcp, mcp_content_retention),
            tool: ToolInteractionProjector::new(config, max_tool_entries_per_trace),
            links: ActionLinkProjector::new(),
        }
    }

    pub fn observe_event(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        self.observe_event_with_diagnostics(event).output
    }

    pub fn observe_event_with_diagnostics(
        &mut self,
        event: &DomainEvent,
    ) -> LiveSemanticActionObservation {
        let mcp_stdio_diagnostics = self.mcp.observe_event(event);
        let output = self.observe_event_semantics(event);
        LiveSemanticActionObservation {
            output,
            mcp_stdio_diagnostics,
        }
    }

    fn observe_event_semantics(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        if matches!(&event.payload, EventPayload::File(_)) {
            return if is_file_modify_event(event) {
                let file_action = file_modify_action(event);
                let file_observation_path = file_action
                    .attributes
                    .get(attrs::file::PATH)
                    .cloned()
                    .map(|path| FileObservationPath {
                        trace_id: file_action.trace_id,
                        action_id: file_action.action_id.clone(),
                        path_order: 0,
                        path,
                    });
                let mut output = self.file_access.observe_file_event(event);
                if !output.raw_event_consumed {
                    let insert_at = output
                        .actions
                        .iter()
                        .take_while(|action| {
                            matches!(
                                action.kind,
                                SemanticActionKind::FileBulkRead | SemanticActionKind::FsEnumerate
                            )
                        })
                        .count();
                    output.actions.insert(insert_at, file_action);
                    if let Some(path) = file_observation_path {
                        output.file_observation_paths.push(path);
                    }
                }
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            } else {
                let mut output = self.file_access.observe_file_event(event);
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            };
        }

        let mut output = if event_projects_semantic_action_boundary(event) {
            self.file_access.observe_boundary_for_event(event)
        } else {
            LiveSemanticActionOutput::default()
        };
        match &event.payload {
            EventPayload::Process(payload) if payload.operation == "exec" => {
                let actions = self.agent.observe_process_exec(event);
                output.actions.extend(actions.clone());
                if let Some(process_action) = actions
                    .iter()
                    .find(|action| action.kind == semantic_action::SemanticActionKind::ProcessExec)
                {
                    output.extend(self.command.observe_process_exec(event, process_action));
                }
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Process(payload) if payload.operation == "fork_attempt" => {
                output.actions.push(process_fork_attempt_action(event));
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Process(payload) if payload.operation == "fork" => {
                self.command.observe_process_fork(event);
                output.links.extend(self.links.observe_process_fork(event));
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Process(payload) if payload.operation == "exit" => {
                output
                    .actions
                    .extend(self.agent.observe_process_exit(event));
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Application(payload) if is_http_protocol(&payload.protocol) => {
                let observation = self
                    .http_exchange
                    .observe_http_message(http_message_action(event));
                let mut projected_llm = LiveSemanticActionOutput::default();
                for matched in &observation.matches {
                    let llm_output = self
                        .llm
                        .advance(PipelineEvent::HttpExchange(matched))
                        .output;
                    projected_llm.extend(self.observe_llm_batch(llm_output));
                }
                for response in &observation.damaged_responses {
                    let llm_output = self
                        .llm
                        .advance(PipelineEvent::DamagedHttpResponse(response))
                        .output;
                    projected_llm.extend(self.observe_llm_batch(llm_output));
                }
                for response in &observation.unmatched_responses {
                    let llm_output = self
                        .llm
                        .advance(PipelineEvent::UnmatchedHttpResponse(response))
                        .output;
                    projected_llm.extend(self.observe_llm_batch(llm_output));
                }
                let llm_actions = std::mem::take(&mut projected_llm.actions);
                output.actions.extend(observation.actions);
                output.actions.extend(llm_actions);
                output.extend(projected_llm);
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Enforcement(_) => {
                output.actions.push(enforcement_action(event));
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            _ => {
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
        }
    }

    pub fn register_llm_codec(
        &mut self,
        plugin: std::sync::Arc<dyn LlmCodecPlugin>,
    ) -> Result<(), String> {
        self.llm.register_codec(plugin)
    }

    pub fn unregister_llm_codec(&mut self, instance_id: &str) -> bool {
        self.llm.unregister_codec(instance_id)
    }

    pub fn llm_codec_statuses(&self) -> Vec<LlmCodecPluginStatus> {
        self.llm.codec_statuses()
    }

    fn observe_llm_batch(&mut self, llm_output: ActionBatch) -> LiveSemanticActionOutput {
        let tool_output = self.tool.project(ToolProjectionBatch {
            actions: &llm_output.actions,
            tool_results: &llm_output.llm_tool_results,
            request_lineages: &llm_output.llm_request_lineages,
        });
        let mut output = LiveSemanticActionOutput {
            payload_segments: llm_output.payload_segments,
            llm_pipeline_diagnostics: llm_output.diagnostics,
            llm_request_contents: llm_output.llm_request_contents,
            llm_request_lineages: llm_output.llm_request_lineages,
            ..LiveSemanticActionOutput::default()
        };
        for proposal in &llm_output.http_request_links {
            if let Some(link) = self.links.observe_exact_http_request_link(proposal) {
                output.links.push(link);
            }
        }
        for proposal in &llm_output.http_response_links {
            if let Some(link) = self.links.observe_exact_http_response_link(proposal) {
                output.links.push(link);
            }
        }
        for mut action in llm_output.actions {
            let agent_actions = if action.kind == SemanticActionKind::LlmRequest {
                self.agent.annotate_user_input(&mut action);
                self.agent.observe_llm_request(&action)
            } else {
                Vec::new()
            };
            output.actions.push(action);
            output.actions.extend(agent_actions);
        }
        output.actions.extend(tool_output.actions);
        output.links.extend(tool_output.links);
        output
            .llm_pipeline_diagnostics
            .extend(tool_output.diagnostics);
        output
    }

    pub fn take_pending_exec_intent_evictions(&mut self) -> u64 {
        self.agent.take_pending_exec_evictions()
    }

    pub fn observe_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        self.observe_payload_segment_with_diagnostics(segment)
            .output
    }

    pub fn observe_payload_segment_with_diagnostics(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionObservation {
        self.observe_payload_segment_with_evidence(segment, true)
    }

    /// Projects an unretained stdio segment only through the local MCP path.
    ///
    /// Payload evidence is omitted because no payload row will be persisted.
    pub fn observe_unretained_mcp_stdio_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        self.observe_unretained_mcp_stdio_payload_segment_with_diagnostics(segment)
            .output
    }

    pub fn observe_unretained_mcp_stdio_payload_segment_with_diagnostics(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionObservation {
        assert!(
            segment.source_boundary == model_core::payload::PayloadSourceBoundary::Stdio,
            "unretained MCP projection requires a stdio payload segment"
        );
        self.observe_payload_segment_with_evidence(segment, false)
    }

    pub fn observe_unretained_payload_segment_with_diagnostics(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionObservation {
        self.observe_payload_segment_with_evidence(segment, false)
    }

    fn observe_payload_segment_with_evidence(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> LiveSemanticActionObservation {
        if retain_evidence {
            self.agent.observe_payload_segment(segment);
        }
        let mut llm_output = self
            .llm
            .advance(PipelineEvent::PayloadSegment(segment))
            .output;
        if !retain_evidence {
            for action in &mut llm_output.actions {
                action
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
        }
        let mut mcp_actions = self.mcp.observe_llm_actions(&llm_output.actions);
        let projected_llm = self.observe_llm_batch(llm_output);
        let (mcp_output, mcp_stdio_diagnostics) =
            self.mcp.observe_payload_segment(segment, retain_evidence);
        mcp_actions.extend(mcp_output.actions);
        let mut output = if projected_llm.actions.is_empty() && mcp_actions.is_empty() {
            LiveSemanticActionOutput::default()
        } else {
            self.file_access.observe_boundary(
                segment.trace_id,
                &segment.process,
                segment.observed_at,
            )
        };
        output.extend(projected_llm);
        for action in &mcp_actions {
            output.extend(self.command.observe_mcp_tool_call(action));
        }
        output.actions.extend(mcp_actions);
        output.links.extend(mcp_output.links);
        output.mcp_jsonrpc_contents.extend(mcp_output.contents);
        output.payload_segments.extend(mcp_output.payload_segments);
        output
            .links
            .extend(self.links.observe_actions(&output.actions));
        if !retain_evidence {
            for action in &mut output.actions {
                action
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
            for link in &mut output.links {
                link.evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
        }
        LiveSemanticActionObservation {
            output,
            mcp_stdio_diagnostics,
        }
    }

    pub fn should_project_dropped_stdio_payload(&self, segment: &PayloadSegment) -> bool {
        self.mcp.should_project_stdio_payload(segment)
    }

    pub fn observe_payload_gap(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionObservation {
        self.http_exchange.quarantine_payload_stream(segment);
        let llm_output = self.llm.advance(PipelineEvent::PayloadGap(segment)).output;
        let mut output = self.observe_llm_batch(llm_output);
        output
            .links
            .extend(self.links.observe_actions(&output.actions));
        LiveSemanticActionObservation {
            output,
            mcp_stdio_diagnostics: Vec::new(),
        }
    }

    pub fn prepare_incomplete_payload(&mut self, segment: &PayloadSegment) {
        self.http_exchange.quarantine_payload_stream(segment);
    }

    pub fn prepare_incomplete_http1_request(
        &mut self,
        segment: &PayloadSegment,
        sequence: u64,
        header_projected: bool,
    ) {
        match self
            .http_exchange
            .observe_damaged_http1_request(segment, sequence, header_projected)
        {
            DamagedHttp1RequestOutcome::Tombstoned => {}
            DamagedHttp1RequestOutcome::MissingPending
                if self
                    .llm
                    .advance(PipelineEvent::LocalizeIncompleteHttp1Request { segment, sequence })
                    .localized => {}
            DamagedHttp1RequestOutcome::MissingPending => {
                self.http_exchange.quarantine_payload_stream(segment);
                self.llm
                    .advance(PipelineEvent::ForgetPayloadAssociations(segment));
            }
            DamagedHttp1RequestOutcome::Unsafe => {
                self.llm
                    .advance(PipelineEvent::ForgetPayloadAssociations(segment));
            }
        }
    }

    pub fn prepare_incomplete_http1_response(
        &mut self,
        segment: &PayloadSegment,
        sequence: u64,
        header_projected: bool,
    ) {
        let request = (!header_projected)
            .then(|| self.http_exchange.observe_damaged_http1_response(segment))
            .flatten();
        self.llm
            .advance(PipelineEvent::PrepareIncompleteHttp1Response {
                segment,
                sequence,
                request,
            });
    }

    pub fn finish_incomplete_payload(&mut self, segment: &PayloadSegment) {
        self.llm
            .advance(PipelineEvent::ForgetPayloadAssociations(segment));
    }

    pub fn finish_incomplete_http1_response(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        let llm_output = self
            .llm
            .advance(PipelineEvent::FinishIncompleteHttp1Response(segment))
            .output;
        let mut output = self.observe_llm_batch(llm_output);
        output
            .links
            .extend(self.links.observe_actions(&output.actions));
        output
    }

    pub fn forget_payload_stream(&mut self, identity: &PayloadStreamIdentity) {
        self.http_exchange.forget_payload_stream(identity);
        self.llm
            .advance(PipelineEvent::ForgetPayloadStream(identity));
    }

    pub fn finalize_payload_stream(
        &mut self,
        identity: &PayloadStreamIdentity,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let llm_output = self
            .llm
            .advance(PipelineEvent::FinalizePayloadStream {
                identity,
                finished_at,
            })
            .output;
        let mut output = self.observe_llm_batch(llm_output);
        output
            .links
            .extend(self.links.observe_actions(&output.actions));
        self.http_exchange.forget_payload_stream(identity);
        output
    }

    pub fn flush_closed_mcp_stdio_sessions(&mut self) {
        let _ = self.flush_closed_mcp_stdio_sessions_with_diagnostics(SystemTime::now());
    }

    pub fn flush_closed_mcp_stdio_sessions_with_diagnostics(
        &mut self,
        emitted_at: SystemTime,
    ) -> Vec<LiveMcpStdioDiagnostic> {
        self.mcp.flush_closed_stdio_sessions(emitted_at)
    }

    pub fn take_mcp_stdio_metrics(&mut self) -> LiveMcpStdioMetrics {
        let metrics = self.mcp.take_stdio_metrics();
        LiveMcpStdioMetrics {
            untracked_stdio: metrics.untracked_stdio,
            candidates: metrics.candidates,
            rejected: metrics.rejected,
            confirmed: metrics.confirmed,
            lifecycle_contract_gaps: metrics.lifecycle_contract_gaps,
            capacity_exhausted: metrics.capacity_exhausted,
            candidate_stream_discards: metrics.candidate_stream_discards,
            confirmed_parse_discards: metrics.confirmed_parse_discards,
            rejection_reasons: metrics
                .rejection_reasons
                .into_iter()
                .map(|(reason, count)| (reason.to_string(), count))
                .collect(),
            discard_reasons: metrics
                .discard_reasons
                .into_iter()
                .map(|(reason, count)| (reason.to_string(), count))
                .collect(),
        }
    }

    pub fn forget_trace(&mut self, trace_id: TraceId) {
        self.agent.forget_trace(trace_id);
        self.command.forget_trace(trace_id);
        self.file_access.forget_trace(trace_id);
        self.http_exchange.forget_trace(trace_id);
        self.llm.advance(PipelineEvent::ForgetTrace(trace_id));
        self.mcp.forget_trace(trace_id);
        self.tool.forget_trace(trace_id);
        self.links.forget_trace(trace_id);
    }

    pub fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let llm_output = self
            .llm
            .advance(PipelineEvent::FinalizeTrace {
                trace_id,
                finished_at,
            })
            .output;
        let mut output = self.observe_llm_batch(llm_output);
        let finalized_tool_output = self.tool.finish_trace(trace_id, finished_at);
        output.actions.extend(finalized_tool_output.actions);
        output
            .llm_pipeline_diagnostics
            .extend(finalized_tool_output.diagnostics);
        output
            .actions
            .extend(self.mcp.finalize_trace(trace_id, finished_at));
        let file_output = self.file_access.finalize_trace(trace_id, finished_at);
        output.actions.extend(file_output.actions);
        output.links.extend(finalized_tool_output.links);
        output
            .links
            .extend(self.links.observe_actions(&output.actions));
        output.file_path_sets.extend(file_output.file_path_sets);
        output.deferred_events.extend(file_output.deferred_events);
        output.retain_event = output.retain_event && file_output.retain_event;
        output
    }
}

fn event_projects_semantic_action_boundary(event: &DomainEvent) -> bool {
    match &event.payload {
        EventPayload::Process(payload) => payload.operation == "exit",
        EventPayload::Application(payload) => is_http_protocol(&payload.protocol),
        EventPayload::Enforcement(_) => true,
        _ => false,
    }
}
