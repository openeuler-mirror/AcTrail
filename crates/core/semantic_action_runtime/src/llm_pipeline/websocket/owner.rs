//! Native Responses message routing and bounded lane state.
use super::session::{Response, Session};
use crate::llm_pipeline::projection::projector::{
    ParsedMessageContext, ParsedMessageProjector, ParsedProviderStream,
};
use crate::llm_pipeline::projection::{ProjectionBatch, ProjectionCoordinator};
use crate::llm_pipeline::transport::websocket::{
    WebSocketAbortReason, WebSocketConnectionKey, WebSocketMessageEvent, WebSocketMessageHead,
};
use crate::llm_pipeline::websocket_source::WebSocketSourceFactory;
use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadStreamIdentity};
use semantic_action::{SemanticActionKind, SemanticActionLinkRole, SemanticEvidence};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::SystemTime;

pub(in crate::llm_pipeline) struct WebSocketProtocol {
    sessions: BTreeMap<WebSocketConnectionKey, Session>,
    sources: WebSocketSourceFactory,
}
impl WebSocketProtocol {
    pub(in crate::llm_pipeline) fn new(config: &SemanticRetentionConfig) -> Self {
        Self {
            sessions: BTreeMap::new(),
            sources: WebSocketSourceFactory::new(config),
        }
    }
    pub(in crate::llm_pipeline) fn observe(
        &mut self,
        events: Vec<WebSocketMessageEvent>,
        config: &SemanticRetentionConfig,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        let mut touched = None;
        for event in events {
            output.extend(self.sources.observe(&event));
            match event {
                WebSocketMessageEvent::Complete {
                    mut head,
                    bytes,
                    evidence,
                    observed_at,
                } => {
                    if !config.llm_layer_enabled()
                        || head.opcode != 1
                        || !head
                            .path
                            .split('?')
                            .next()
                            .is_some_and(|path| path.ends_with("/responses"))
                    {
                        continue;
                    }
                    head.evidence = evidence;
                    let key = WebSocketConnectionKey(head.identity.clone());
                    let session = self.sessions.entry(key.clone()).or_default();
                    touched = Some(key);
                    if session.blocked {
                        continue;
                    }
                    let value: Value = match serde_json::from_slice(&bytes) {
                        Ok(value) => value,
                        Err(_) => {
                            output
                                .extend(session.abort(config, WebSocketAbortReason::DecodeFailure));
                            continue;
                        }
                    };
                    let byte_len = bytes.len();
                    drop(bytes);
                    Self::message(
                        session,
                        &self.sources,
                        head,
                        observed_at,
                        byte_len,
                        value,
                        config,
                        &mut output,
                    );
                }
                WebSocketMessageEvent::Abort { head, reason, .. } => {
                    touched = Some(WebSocketConnectionKey(head.identity.clone()));
                    if let Some(session) = self
                        .sessions
                        .get_mut(&WebSocketConnectionKey(head.identity))
                    {
                        output.extend(session.abort(config, reason));
                    }
                }
                WebSocketMessageEvent::Start(_) => {}
            }
        }
        let mut bindings = Vec::new();
        if let Some(key) = touched {
            if let Some(session) = self.sessions.get_mut(&key) {
                session.flush(config, &mut output);
                bindings = std::mem::take(&mut session.response_bindings);
            }
        }
        let mut output = projection.changed_actions(config, output);
        for binding in bindings {
            output.extend(projection.observe_response_binding(binding));
        }
        output
    }
    fn message(
        session: &mut Session,
        sources: &WebSocketSourceFactory,
        head: WebSocketMessageHead,
        at: SystemTime,
        size: usize,
        value: Value,
        config: &SemanticRetentionConfig,
        output: &mut ProjectionBatch,
    ) {
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        let requested_lane = value
            .get("stream_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if value.get("stream_id").is_some()
            && (requested_lane.is_empty()
                || requested_lane.len() > 256
                || !requested_lane
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
        {
            output.extend(session.abort(config, WebSocketAbortReason::DecodeFailure));
            return;
        }
        let provider_id = value
            .get("response_id")
            .or_else(|| {
                value
                    .get("response")
                    .and_then(|response| response.get("id"))
            })
            .and_then(Value::as_str)
            .map(str::to_owned);
        if head.direction == PayloadDirection::Outbound {
            if event_type != "response.create" {
                return;
            }
            if session
                .lanes
                .get(&requested_lane)
                .is_some_and(|lane| lane.blocked)
            {
                return;
            }
            if session.request_count
                >= config
                    .l0_llm_call
                    .websocket
                    .max_pending_requests_per_connection as usize
                || (!session.lanes.contains_key(&requested_lane)
                    && session.lanes.len()
                        >= config.l0_llm_call.websocket.max_lanes_per_connection as usize)
            {
                output.extend(session.abort(config, WebSocketAbortReason::CapacityExceeded));
                return;
            }
            let context = Self::context(&head, at, size, "llm.request");
            let Some(mut projected) = ParsedMessageProjector::request(config, &context, value)
            else {
                session
                    .lanes
                    .entry(requested_lane)
                    .or_default()
                    .pending
                    .push_back(None);
                session.request_count += 1;
                return;
            };
            let Some(request) = projected
                .actions
                .iter_mut()
                .find(|action| action.kind == SemanticActionKind::LlmRequest)
            else {
                session
                    .lanes
                    .entry(requested_lane)
                    .or_default()
                    .pending
                    .push_back(None);
                session.request_count += 1;
                return;
            };
            request
                .attributes
                .insert("websocket.connection_id".into(), head.connection_id.clone());
            request
                .attributes
                .insert("websocket.message_id".into(), head.message_id.clone());
            request
                .attributes
                .insert("websocket.stream_id".into(), requested_lane.clone());
            let request = crate::live::action_for_live_state(request);
            output.extend(sources.bind(
                &request.action_id,
                &head,
                SemanticActionLinkRole::LlmRequestWebSocketMessage,
            ));
            Session::call(&request, None, output);
            session
                .lanes
                .entry(requested_lane)
                .or_default()
                .pending
                .push_back(Some(request));
            session.request_count += 1;
            output.extend(projected);
            return;
        }
        if !event_type.starts_with("response.") && event_type != "error" {
            return;
        }
        let bound_lane = provider_id.as_ref().and_then(|id| {
            session
                .lanes
                .iter()
                .find(|(_, lane)| {
                    lane.active
                        .as_ref()
                        .is_some_and(|active| active.provider_id == *id)
                })
                .map(|(name, _)| name.clone())
        });
        let unknown_response =
            event_type != "response.created" && provider_id.is_some() && bound_lane.is_none();
        let lane_name = bound_lane.unwrap_or_else(|| requested_lane.clone());
        if value.get("stream_id").is_some() && lane_name != requested_lane {
            output.extend(session.abort(config, WebSocketAbortReason::DecodeFailure));
            return;
        }
        if !session.lanes.contains_key(&lane_name)
            && session.lanes.len() >= config.l0_llm_call.websocket.max_lanes_per_connection as usize
        {
            output.extend(session.abort(config, WebSocketAbortReason::CapacityExceeded));
            return;
        }
        let lane = session.lanes.entry(lane_name.clone()).or_default();
        if lane.blocked {
            return;
        }
        if unknown_response {
            // Responses events without stream_id belong to the protocol's default
            // lane. An unknown response breaks this lane's create ordering only.
            output.extend(session.abort_lane(
                &lane_name,
                config,
                WebSocketAbortReason::DecodeFailure,
            ));
            return;
        }
        if lane.active.is_none() && size > config.l0_llm_call.assembly.max_buffer_bytes as usize {
            output.extend(session.abort_lane(
                &lane_name,
                config,
                WebSocketAbortReason::CapacityExceeded,
            ));
            return;
        }
        let mut orphan = false;
        if event_type == "response.created" {
            if lane.active.is_some() {
                output.extend(session.abort(config, WebSocketAbortReason::DecodeFailure));
                return;
            }
            let Some(id) = provider_id else {
                output.extend(session.abort_lane(
                    &lane_name,
                    config,
                    WebSocketAbortReason::DecodeFailure,
                ));
                return;
            };
            let Some(parser) = ParsedProviderStream::new(config, &value) else {
                output.extend(session.abort_lane(
                    &lane_name,
                    config,
                    WebSocketAbortReason::DecodeFailure,
                ));
                return;
            };
            let pending = lane.pending.pop_front();
            lane.active = Some(Response {
                has_create: pending.is_some(),
                request: pending.flatten(),
                provider_id: id,
                parser,
                context: Self::context(&head, at, size, "llm.response"),
                first: head.clone(),
                last: head.clone(),
                message_count: 1,
                dirty: true,
            });
            if let Some(active) = &lane.active {
                active.bind_call(output);
            }
        } else if let Some(active) = lane.active.as_mut() {
            if provider_id
                .as_ref()
                .is_some_and(|id| *id != active.provider_id)
            {
                return;
            }
            if active.context.payload_bytes.saturating_add(size)
                > config.l0_llm_call.assembly.max_buffer_bytes as usize
            {
                output.extend(sources.bind(
                    &active.context.action_id,
                    &head,
                    SemanticActionLinkRole::LlmResponseWebSocketMessage,
                ));
                active.context.last_at = at;
                active.context.evidence = Self::boundary_evidence(&active.first, &head);
                active.last = head;
                active.message_count += 1;
                output.extend(session.abort_lane(
                    &lane_name,
                    config,
                    WebSocketAbortReason::CapacityExceeded,
                ));
                return;
            }
            active.parser.observe(&value);
            active.context.last_at = at;
            active.context.payload_bytes = active.context.payload_bytes.saturating_add(size);
            active.context.evidence = Self::boundary_evidence(&active.first, &head);
            active.last = head.clone();
            active.message_count += 1;
            active.dirty |= active.parser.terminal();
        } else {
            // Named request-scoped errors obey the lane's serial request order.
            orphan = event_type != "error" || value.get("stream_id").is_none();
            let parser = if event_type == "error" {
                let mut parser = ParsedProviderStream::responses(config);
                parser.observe(&value);
                parser
            } else {
                let Some(parser) = ParsedProviderStream::new(config, &value) else {
                    output.extend(session.abort_lane(
                        &lane_name,
                        config,
                        WebSocketAbortReason::DecodeFailure,
                    ));
                    return;
                };
                parser
            };
            let pending = if event_type == "error" && value.get("stream_id").is_some() {
                lane.pending.pop_front()
            } else {
                None
            };
            lane.active = Some(Response {
                has_create: pending.is_some(),
                request: pending.flatten(),
                provider_id: provider_id.unwrap_or_default(),
                parser,
                context: Self::context(&head, at, size, "llm.response"),
                first: head.clone(),
                last: head.clone(),
                message_count: 1,
                dirty: true,
            });
            if let Some(active) = &lane.active {
                active.bind_call(output);
            }
        }
        if let Some(active) = &lane.active {
            output.extend(sources.bind(
                &active.context.action_id,
                &head,
                SemanticActionLinkRole::LlmResponseWebSocketMessage,
            ));
        }
        if orphan {
            lane.blocked = true;
            if lane
                .active
                .as_ref()
                .is_some_and(|active| active.parser.terminal())
            {
                session.flush(config, output);
            }
            output.extend(session.abort_lane(
                &lane_name,
                config,
                WebSocketAbortReason::DecodeFailure,
            ));
            return;
        }
        // A terminal frees the lane before the next message in this same capture batch.
        if lane
            .active
            .as_ref()
            .is_some_and(|active| active.parser.terminal())
        {
            session.flush(config, output);
        }
    }
    fn context(
        head: &WebSocketMessageHead,
        at: SystemTime,
        size: usize,
        suffix: &str,
    ) -> ParsedMessageContext {
        ParsedMessageContext {
            action_id: format!("{}:{suffix}", head.message_id),
            identity: head.identity.clone(),
            start_at: head.start_at,
            last_at: at,
            evidence: Self::boundary_evidence(head, head),
            payload_bytes: size,
        }
    }
    fn boundary_evidence(
        first: &WebSocketMessageHead,
        last: &WebSocketMessageHead,
    ) -> Vec<SemanticEvidence> {
        let mut evidence = first
            .evidence
            .first()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(last) = last.evidence.last() {
            if !evidence.contains(last) {
                evidence.push(last.clone());
            }
        }
        evidence
    }
    pub(in crate::llm_pipeline) fn abort_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
        reason: WebSocketAbortReason,
        config: &SemanticRetentionConfig,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let Some(mut session) = self
            .sessions
            .remove(&WebSocketConnectionKey(identity.clone()))
        else {
            return ProjectionBatch::default();
        };
        let mut output = projection.changed_actions(config, session.abort(config, reason));
        for binding in session.response_bindings {
            output.extend(projection.observe_response_binding(binding));
        }
        output
    }
    pub(in crate::llm_pipeline) fn forget_identity(&mut self, identity: &PayloadStreamIdentity) {
        self.sessions
            .remove(&WebSocketConnectionKey(identity.clone()));
    }
    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        self.sessions.retain(|key, _| key.0.trace_id != trace_id);
    }
}
