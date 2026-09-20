//! Live semantic action runtime.
use config_core::daemon::{
    AgentInvocationConfig, FileObservationConfig, PayloadMcpConfig, SemanticRetentionConfig,
};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use semantic_action::{FileObservationPath, SemanticActionKind, attr_keys as attrs};
use std::time::SystemTime;

use crate::llm_pipeline::{
    ActionBatch, LlmActionPipeline, LlmCodecPlugin, LlmCodecPluginStatus, PipelineEvent,
};

use super::actions::{
    enforcement_action, file_modify_action, http_message_action, is_http_protocol,
    process_fork_attempt_action,
};
use super::agent::AgentProjector;
use super::command::CommandProjector;
use super::file::FileAccessProjector;
use super::http_exchange::HttpExchangeTracker;
use super::links::ActionLinkProjector;
use super::mcp::{LiveMcpProjector, LiveMcpStdioDiagnostic};
use super::tool::{ToolInteractionProjector, ToolProjectionBatch};

pub struct LiveSemanticActionRuntime {
    projection_enabled: bool,
    agent: AgentProjector,
    command: CommandProjector,
    file_access: FileAccessProjector,
    http_exchange: HttpExchangeTracker,
    llm: LlmActionPipeline,
    mcp: LiveMcpProjector,
    tool: ToolInteractionProjector,
    links: ActionLinkProjector,
}

mod output;
mod payload;
pub use output::{LiveMcpStdioMetrics, LiveSemanticActionObservation, LiveSemanticActionOutput};

impl LiveSemanticActionRuntime {
    pub fn finish_file_io_batch(&mut self) -> LiveSemanticActionOutput {
        let mut output = self.file_access.finish_file_io_batch();
        output
            .links
            .extend(self.links.observe_actions(output.actions.iter()));
        output
    }
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
            projection_enabled: semantic_retention.projection_enabled,
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
        if !self.projection_enabled {
            return LiveSemanticActionObservation {
                output: LiveSemanticActionOutput::default(),
                mcp_stdio_diagnostics: Vec::new(),
            };
        }
        let mcp_stdio_diagnostics = self.mcp.observe_event(event);
        let output = self.observe_event_semantics(event);
        LiveSemanticActionObservation {
            output,
            mcp_stdio_diagnostics,
        }
    }

    fn observe_event_semantics(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        if matches!(&event.payload, EventPayload::File(_)) {
            let mut output = self.file_access.observe_file_event(event);
            if !output.raw_event_consumed && self.file_access.projects_modify_event(event) {
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
            output.links.extend(
                self.links
                    .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
            );
            return output;
        }

        let mut output = if event_projects_semantic_action_boundary(event) {
            self.file_access.observe_boundary_for_event(event)
        } else {
            LiveSemanticActionOutput::default()
        };
        match &event.payload {
            EventPayload::Process(payload) if payload.operation == "exec" => {
                let actions = self.agent.observe_process_exec(event);
                let command_output = actions
                    .iter()
                    .find(|action| action.kind == semantic_action::SemanticActionKind::ProcessExec)
                    .map(|process_action| {
                        self.command
                            .observe_process_exec(event, process_action, self.mcp.enabled())
                    });
                output.actions.extend(actions);
                if let Some(command_output) = command_output {
                    output.extend(command_output);
                }
                output.links.extend(
                    self.links.observe_actions(
                        output.actions.iter().chain(output.updated_actions.iter()),
                    ),
                );
                output
            }
            EventPayload::Process(payload) if payload.operation == "fork_attempt" => {
                output.actions.push(process_fork_attempt_action(event));
                output.links.extend(
                    self.links.observe_actions(
                        output.actions.iter().chain(output.updated_actions.iter()),
                    ),
                );
                output
            }
            EventPayload::Process(payload) if payload.operation == "fork" => {
                self.command.observe_process_fork(event);
                output.links.extend(self.links.observe_process_fork(event));
                output.links.extend(
                    self.links.observe_actions(
                        output.actions.iter().chain(output.updated_actions.iter()),
                    ),
                );
                output
            }
            EventPayload::Process(payload) if payload.operation == "exit" => {
                output
                    .actions
                    .extend(self.agent.observe_process_exit(event));
                output.links.extend(
                    self.links.observe_actions(
                        output.actions.iter().chain(output.updated_actions.iter()),
                    ),
                );
                output
            }
            EventPayload::Application(payload) if is_http_protocol(&payload.protocol) => {
                let observation = self
                    .http_exchange
                    .observe_http_message(http_message_action(event));
                let mut projected_llm = LiveSemanticActionOutput::default();
                for matched in &observation.matches {
                    if matched.reconciled_late {
                        output.updated_actions.push(matched.response.clone());
                    }
                    output
                        .links
                        .push(self.links.observe_exact_http_exchange_link(matched));
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
                output.links.extend(
                    self.links.observe_actions(
                        output.actions.iter().chain(output.updated_actions.iter()),
                    ),
                );
                output
            }
            EventPayload::Enforcement(_) => {
                output.actions.push(enforcement_action(event));
                output.links.extend(
                    self.links.observe_actions(
                        output.actions.iter().chain(output.updated_actions.iter()),
                    ),
                );
                output
            }
            _ => {
                output.links.extend(
                    self.links.observe_actions(
                        output.actions.iter().chain(output.updated_actions.iter()),
                    ),
                );
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
            updated_actions: &llm_output.updated_actions,
            tool_results: &llm_output.llm_tool_results,
            request_lineages: &llm_output.llm_request_lineages,
        });
        let mut output = LiveSemanticActionOutput {
            payload_segments: llm_output.payload_segments,
            updates: llm_output.updates,
            updated_actions: llm_output.updated_actions,
            links: llm_output.links,
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
        output.updates.extend(tool_output.updates);
        output.updated_actions.extend(tool_output.updated_actions);
        output.links.extend(tool_output.links);
        output
            .llm_pipeline_diagnostics
            .extend(tool_output.diagnostics);
        output
    }

    pub fn take_pending_exec_intent_evictions(&mut self) -> u64 {
        self.agent.take_pending_exec_evictions()
    }

    pub fn flush_closed_mcp_stdio_sessions(&mut self) {
        let _ = self.flush_closed_mcp_stdio_sessions_with_diagnostics(SystemTime::now());
    }

    pub fn flush_closed_mcp_stdio_sessions_with_diagnostics(
        &mut self,
        emitted_at: SystemTime,
    ) -> Vec<LiveMcpStdioDiagnostic> {
        if !self.projection_enabled {
            return Vec::new();
        }
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
        if !self.projection_enabled {
            return LiveSemanticActionOutput::default();
        }
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
        output.updates.extend(finalized_tool_output.updates);
        output
            .updated_actions
            .extend(finalized_tool_output.updated_actions);
        output
            .llm_pipeline_diagnostics
            .extend(finalized_tool_output.diagnostics);
        let mcp_output = self.mcp.finalize_trace(trace_id, finished_at);
        output.actions.extend(mcp_output.actions);
        output.updates.extend(mcp_output.updates);
        output.updated_actions.extend(mcp_output.updated_actions);
        let file_output = self.file_access.finalize_trace(trace_id, finished_at);
        output.actions.extend(file_output.actions);
        output.links.extend(finalized_tool_output.links);
        output.links.extend(
            self.links
                .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
        );
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
