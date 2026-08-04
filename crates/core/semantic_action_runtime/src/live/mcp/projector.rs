use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{L0McpCallRetention, PayloadMcpConfig};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessIdentity;
use semantic_action::{
    McpJsonRpcContentWrite, SemanticAction, SemanticActionLink, attr_keys as attrs,
};

use super::attribution::McpAttributionState;
use super::content::McpJsonRpcContentProjector;
use super::diagnostic::{LiveMcpStdioDiagnostic, McpStdioMetrics};
use super::model::{McpCorrelationState, McpServerState, McpStdioSessionKey};
use super::session::McpStdioSessionRegistry;

pub(in crate::live) struct LiveMcpProjector {
    pub(super) enabled: bool,
    pub(super) sessions: McpStdioSessionRegistry,
    pub(super) servers: BTreeMap<McpStdioSessionKey, McpServerState>,
    pub(super) clients: BTreeMap<McpStdioSessionKey, ProcessIdentity>,
    pub(super) exec_parents: BTreeMap<McpStdioSessionKey, ProcessIdentity>,
    pub(super) correlation: McpCorrelationState,
    pub(super) attribution: McpAttributionState,
    pub(super) content: McpJsonRpcContentProjector,
}

#[derive(Default)]
pub(in crate::live) struct McpProjectionOutput {
    pub(in crate::live) actions: Vec<SemanticAction>,
    pub(in crate::live) links: Vec<SemanticActionLink>,
    pub(in crate::live) contents: Vec<McpJsonRpcContentWrite>,
}

impl McpProjectionOutput {
    pub(super) fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.links.extend(other.links);
        self.contents.extend(other.contents);
    }
}

impl LiveMcpProjector {
    pub(in crate::live) fn new(
        config: PayloadMcpConfig,
        content_retention: L0McpCallRetention,
    ) -> Self {
        assert!(
            config.parse_buffer_max_bytes != 0
                && config.stdio_candidate_max_bytes != 0
                && config.pending_stdio_candidate_max_entries != 0,
            "MCP stdio limits must be positive"
        );
        let parse_buffer_max_bytes = usize::try_from(config.parse_buffer_max_bytes)
            .expect("validated MCP parse buffer limit must fit usize");
        let candidate_max_bytes = usize::try_from(config.stdio_candidate_max_bytes)
            .expect("validated MCP stdio candidate limit must fit usize");
        assert!(
            candidate_max_bytes <= parse_buffer_max_bytes,
            "MCP stdio candidate limit must not exceed parse buffer limit"
        );
        Self {
            enabled: config.enabled,
            sessions: McpStdioSessionRegistry::new(
                candidate_max_bytes,
                parse_buffer_max_bytes,
                config.pending_stdio_candidate_max_entries as usize,
            ),
            servers: BTreeMap::new(),
            clients: BTreeMap::new(),
            exec_parents: BTreeMap::new(),
            correlation: McpCorrelationState::default(),
            attribution: McpAttributionState::default(),
            content: McpJsonRpcContentProjector::new(content_retention),
        }
    }

    pub(in crate::live) fn observe_event(
        &mut self,
        event: &DomainEvent,
    ) -> Vec<LiveMcpStdioDiagnostic> {
        if !self.enabled {
            return Vec::new();
        }
        let lifecycle = self.sessions.observe_lifecycle_event(event);
        for key in &lifecycle.removed_sessions {
            self.clear_protocol_state(&key);
            self.clients.remove(&key);
            self.exec_parents.remove(&key);
        }
        if let Some(key) = lifecycle.bound_session
            && let EventPayload::Ipc(payload) = &event.payload
            && let Some(client) = payload
                .metadata
                .get(attrs::mcp::CLIENT_PROCESS_ID)
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value != 0)
        {
            self.clients.insert(key, ProcessIdentity::new(client));
        }

        let EventPayload::Process(payload) = &event.payload else {
            return lifecycle.diagnostics;
        };
        if !matches!(payload.operation.as_str(), "exec" | "exit") {
            return lifecycle.diagnostics;
        }
        if payload.operation == "exit" {
            self.sessions
                .close_process(event.envelope.trace_id, event.envelope.process.clone());
            return lifecycle.diagnostics;
        }

        if let Some(key) = self
            .sessions
            .session_for_process(event.envelope.trace_id, &event.envelope.process)
            && let Some(parent) = &payload.parent
        {
            self.exec_parents.insert(key, parent.clone());
        }
        lifecycle.diagnostics
    }

    pub(in crate::live) fn observe_payload_segment(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> (McpProjectionOutput, Vec<LiveMcpStdioDiagnostic>) {
        if !self.enabled {
            return (McpProjectionOutput::default(), Vec::new());
        }
        let route = self.sessions.route_segment(segment, retain_evidence);
        let mut output = McpProjectionOutput::default();
        if let Some(session) = route.session {
            for buffered in route.messages {
                output.extend(self.observe_message(&session, buffered));
            }
        }
        (output, route.diagnostics)
    }

    pub(in crate::live) fn should_project_stdio_payload(&self, segment: &PayloadSegment) -> bool {
        self.enabled && self.sessions.should_route(segment)
    }

    pub(in crate::live) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> Vec<SemanticAction> {
        let request_keys = self
            .correlation
            .open_calls
            .keys()
            .filter(|key| key.response.session.trace_id == trace_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut actions = Vec::with_capacity(request_keys.len());
        for request_key in request_keys {
            let Some(mut open) = self.correlation.open_calls.remove(&request_key) else {
                continue;
            };
            open.action.status = semantic_action::SemanticActionStatus::Error;
            open.action.completeness = semantic_action::SemanticActionCompleteness::Partial;
            open.action.end_time = Some(finished_at);
            open.action.attributes.insert(
                attrs::mcp::EXECUTION_STATUS.to_string(),
                semantic_action::SemanticActionStatus::Error
                    .as_str()
                    .to_string(),
            );
            open.action.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                "true".to_string(),
            );
            actions.push(open.action);
        }
        self.forget_trace(trace_id);
        actions
    }

    pub(in crate::live) fn flush_closed_stdio_sessions(
        &mut self,
        emitted_at: SystemTime,
    ) -> Vec<LiveMcpStdioDiagnostic> {
        let drain = self.sessions.take_closed_sessions(emitted_at);
        for key in drain.sessions {
            self.clear_protocol_state(&key);
            self.clients.remove(&key);
            self.exec_parents.remove(&key);
        }
        drain.diagnostics
    }

    pub(in crate::live) fn forget_trace(&mut self, trace_id: TraceId) {
        self.sessions.forget_trace(trace_id);
        self.servers.retain(|key, _| key.trace_id != trace_id);
        self.clients.retain(|key, _| key.trace_id != trace_id);
        self.exec_parents.retain(|key, _| key.trace_id != trace_id);
        self.correlation
            .invocation_sequences
            .retain(|key, _| key.session.trace_id != trace_id);
        self.correlation
            .open_calls
            .retain(|key, _| key.response.session.trace_id != trace_id);
        self.correlation
            .open_by_response
            .retain(|key, _| key.session.trace_id != trace_id);
        self.forget_attribution_trace(trace_id);
    }

    pub(in crate::live) fn take_stdio_metrics(&mut self) -> McpStdioMetrics {
        self.sessions.take_metrics()
    }

    pub(super) fn parent_process(&self, session: &McpStdioSessionKey) -> Option<&ProcessIdentity> {
        self.clients
            .get(session)
            .or_else(|| self.exec_parents.get(session))
    }

    fn clear_protocol_state(&mut self, session: &McpStdioSessionKey) {
        self.servers.remove(session);
        self.correlation
            .invocation_sequences
            .retain(|key, _| &key.session != session);
        self.correlation
            .open_calls
            .retain(|key, _| &key.response.session != session);
        self.correlation
            .open_by_response
            .retain(|key, _| &key.session != session);
    }
}
