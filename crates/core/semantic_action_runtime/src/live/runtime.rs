//! Live semantic action runtime.

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use config_core::daemon::{
    AgentInvocationConfig, FileObservationConfig, PayloadMcpConfig, SemanticRetentionConfig,
};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessIdentity;
use semantic_action::{
    FileObservationPath, FilePathSetWrite, LlmRequestContentWrite, McpJsonRpcContentWrite,
    SemanticAction, SemanticActionKind, SemanticActionLink, attr_keys as attrs,
};

use crate::payload_projection::llm::{LlmCodecPlugin, LlmCodecPluginStatus};

use super::actions::{
    enforcement_action, file_modify_action, http_message_action, is_file_modify_event,
    is_http_protocol, process_fork_attempt_action,
};
use super::agent::AgentProjector;
use super::command::CommandProjector;
use super::file::FileAccessProjector;
use super::links::ActionLinkProjector;
use super::llm::{LiveLlmOutput, LiveLlmProjector};
use super::mcp::{LiveMcpProjector, LiveMcpStdioDiagnostic};

const HTTP_DIRECTION_ATTR: &str = "direction";
const HTTP_PAYLOAD_SEQUENCE_ATTR: &str = "payload_sequence";
const HTTP_STATUS_CODE_ATTR: &str = "status_code";
const HTTP_STREAM_ID_ATTR: &str = "stream_id";
const HTTP_STREAM_KEY_ATTR: &str = "stream_key";

pub struct LiveSemanticActionRuntime {
    agent: AgentProjector,
    command: CommandProjector,
    file_access: FileAccessProjector,
    http_exchange: HttpExchangeTracker,
    llm: LiveLlmProjector,
    mcp: LiveMcpProjector,
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
    pub mcp_jsonrpc_contents: Vec<McpJsonRpcContentWrite>,
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
            mcp_jsonrpc_contents: Vec::new(),
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
        self.mcp_jsonrpc_contents.extend(other.mcp_jsonrpc_contents);
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
        let AgentInvocationConfig {
            enabled,
            commands: _,
        } = config;
        let mcp_content_retention = semantic_retention.l0_mcp_call.clone();
        Self {
            agent: AgentProjector::new(enabled, pending_exec_max_entries),
            command: CommandProjector::new(),
            file_access: FileAccessProjector::new(file_observation),
            http_exchange: HttpExchangeTracker::default(),
            llm: LiveLlmProjector::new(semantic_retention),
            mcp: LiveMcpProjector::new(mcp, mcp_content_retention),
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
                let mut action = http_message_action(event);
                self.http_exchange.observe_http_message(&mut action);
                output.actions.push(action.clone());
                output
                    .actions
                    .extend(self.llm.observe_http_message(&action));
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

    fn observe_payload_segment_with_evidence(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> LiveSemanticActionObservation {
        let llm_output = if retain_evidence {
            self.llm.observe_payload_segment(segment)
        } else {
            LiveLlmOutput::default()
        };
        let mut mcp_actions = self.mcp.observe_llm_actions(&llm_output.actions);
        let (mcp_output, mcp_stdio_diagnostics) =
            self.mcp.observe_payload_segment(segment, retain_evidence);
        mcp_actions.extend(mcp_output.actions);
        let mut output = if llm_output.actions.is_empty() && mcp_actions.is_empty() {
            LiveSemanticActionOutput::default()
        } else {
            self.file_access.observe_boundary(
                segment.trace_id,
                &segment.process,
                segment.observed_at,
            )
        };
        output
            .llm_request_contents
            .extend(llm_output.llm_request_contents);
        for action in llm_output.actions {
            let agent_actions = if action.kind == SemanticActionKind::LlmRequest {
                self.agent.observe_llm_request(&action)
            } else {
                Vec::new()
            };
            output.actions.push(action.clone());
            output.actions.extend(agent_actions);
        }
        for action in &mcp_actions {
            output.extend(self.command.observe_mcp_tool_call(action));
        }
        output.actions.extend(mcp_actions);
        output.links.extend(mcp_output.links);
        output.mcp_jsonrpc_contents.extend(mcp_output.contents);
        output
            .links
            .extend(self.links.observe_actions(&output.actions));
        LiveSemanticActionObservation {
            output,
            mcp_stdio_diagnostics,
        }
    }

    pub fn should_project_dropped_stdio_payload(&self, segment: &PayloadSegment) -> bool {
        self.mcp.should_project_stdio_payload(segment)
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
        self.llm.forget_trace(trace_id);
        self.mcp.forget_trace(trace_id);
        self.links.forget_trace(trace_id);
    }

    pub fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let mut actions = self.llm.finalize_trace(trace_id, finished_at);
        actions.extend(self.mcp.finalize_trace(trace_id, finished_at));
        let file_output = self.file_access.finalize_trace(trace_id, finished_at);
        actions.extend(file_output.actions);
        let links = self.links.observe_actions(&actions);
        LiveSemanticActionOutput {
            actions,
            links,
            file_observation_paths: Vec::new(),
            file_path_sets: file_output.file_path_sets,
            llm_request_contents: Vec::new(),
            mcp_jsonrpc_contents: Vec::new(),
            deferred_events: file_output.deferred_events,
            retain_event: file_output.retain_event,
            raw_event_consumed: false,
        }
    }
}

#[derive(Default)]
struct HttpExchangeTracker {
    pending_by_stream: BTreeMap<HttpExchangeKey, VecDeque<PendingHttpRequest>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct HttpExchangeKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: String,
    stream_id: Option<String>,
}

#[derive(Clone, Debug)]
struct PendingHttpRequest {
    action_id: String,
    sequence: u64,
}

impl HttpExchangeTracker {
    fn observe_http_message(&mut self, action: &mut SemanticAction) {
        match http_message_direction_operation(action) {
            Some(("outbound", "request")) => self.observe_request(action),
            Some(("inbound", "response")) => self.annotate_response(action),
            _ => {}
        }
    }

    fn forget_trace(&mut self, trace_id: TraceId) {
        self.pending_by_stream
            .retain(|key, _| key.trace_id != trace_id);
    }

    fn observe_request(&mut self, action: &SemanticAction) {
        let Some(key) = HttpExchangeKey::from_http_message(action) else {
            return;
        };
        let Some(sequence) = http_payload_sequence(action) else {
            return;
        };
        self.pending_by_stream
            .entry(key)
            .or_default()
            .push_back(PendingHttpRequest {
                action_id: action.action_id.clone(),
                sequence,
            });
    }

    fn annotate_response(&mut self, action: &mut SemanticAction) {
        let Some(status_code) = http_status_code(action) else {
            return;
        };
        let Some(response_sequence) = http_payload_sequence(action) else {
            return;
        };
        let Some(key) = HttpExchangeKey::from_http_message(action) else {
            return;
        };
        let Some(requests) = self.pending_by_stream.get_mut(&key) else {
            return;
        };
        let Some(request) = requests.front() else {
            return;
        };
        if request.sequence > response_sequence {
            return;
        }
        action.attributes.insert(
            attrs::http_response::REQUEST_ACTION_ID.to_string(),
            request.action_id.clone(),
        );
        if final_http_response(status_code) {
            requests.pop_front();
        }
        if requests.is_empty() {
            self.pending_by_stream.remove(&key);
        }
    }
}

impl HttpExchangeKey {
    fn from_http_message(action: &SemanticAction) -> Option<Self> {
        if action.kind != SemanticActionKind::HttpMessage {
            return None;
        }
        Some(Self {
            trace_id: action.trace_id,
            process: action.process.clone(),
            stream_key: action.attributes.get(HTTP_STREAM_KEY_ATTR)?.clone(),
            stream_id: action.attributes.get(HTTP_STREAM_ID_ATTR).cloned(),
        })
    }
}

fn http_message_direction_operation(action: &SemanticAction) -> Option<(&str, &str)> {
    Some((
        action.attributes.get(HTTP_DIRECTION_ATTR)?.as_str(),
        action.attributes.get(attrs::http::OPERATION)?.as_str(),
    ))
}

fn http_payload_sequence(action: &SemanticAction) -> Option<u64> {
    action
        .attributes
        .get(HTTP_PAYLOAD_SEQUENCE_ATTR)?
        .parse()
        .ok()
}

fn http_status_code(action: &SemanticAction) -> Option<u16> {
    action.attributes.get(HTTP_STATUS_CODE_ATTR)?.parse().ok()
}

fn final_http_response(status_code: u16) -> bool {
    !(100..=199).contains(&status_code) || status_code == 101
}

fn event_projects_semantic_action_boundary(event: &DomainEvent) -> bool {
    match &event.payload {
        EventPayload::Process(payload) => payload.operation == "exit",
        EventPayload::Application(payload) => is_http_protocol(&payload.protocol),
        EventPayload::Enforcement(_) => true,
        _ => false,
    }
}
