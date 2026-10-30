//! HTTP/1 message ownership before application and LLM projection.

use crate::live::HttpExchangeTracker;
use crate::llm_pipeline::assembly::router::{
    LiveStreamDirection, LiveStreamKey, PayloadStreamGroupKey,
};
use crate::llm_pipeline::http::HttpMessageFacts;
use crate::llm_pipeline::projection::projector::{
    project_decoded_http1_request, project_decoded_http1_response,
};
use crate::llm_pipeline::projection::{ProjectionBatch, ProjectionCoordinator};
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::response::IncrementalSseCache;
use crate::llm_pipeline::transport::http1::{Http1Session, Http1Update, Http1Upgrade};
use crate::{Http1AbortReason, Http1MessageEvent};
use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use semantic_action::SemanticAction;
use std::collections::BTreeMap;
use std::time::SystemTime;

pub(in crate::llm_pipeline) struct Http1Protocol {
    sessions: BTreeMap<LiveStreamKey, Http1Session>,
    next_message: u64,
    exchange: HttpExchangeTracker,
    facts: HttpMessageFacts,
    sse: BTreeMap<String, Option<IncrementalSseCache>>,
    max_bytes: usize,
    max_decoded_bytes: usize,
    max_ranges: usize,
    max_sessions: usize,
    resumed: Vec<(String, String)>,
    retain_http: bool,
}

impl Http1Protocol {
    pub(in crate::llm_pipeline) fn new(config: &SemanticRetentionConfig) -> Self {
        Self {
            sessions: BTreeMap::new(),
            next_message: 0,
            exchange: HttpExchangeTracker::new(config.l2_http.exchange),
            facts: HttpMessageFacts::new(
                config.l2_http.exchange.max_message_facts_per_trace as usize,
            ),
            sse: BTreeMap::new(),
            max_bytes: config.l2_http.http1.max_buffer_bytes as usize,
            max_decoded_bytes: config.l0_llm_call.assembly.max_buffer_bytes as usize,
            max_ranges: config.l2_http.http1.max_evidence_ranges as usize,
            max_sessions: config.l2_http.max_active_directions as usize,
            resumed: Vec::new(),
            retain_http: config.http_message_summary_enabled(),
        }
    }

    pub(in crate::llm_pipeline) fn advance(
        &mut self,
        segment: &PayloadSegment,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let key = LiveStreamKey::from_segment(segment);
        let mut output = ProjectionBatch {
            http1_handled: true,
            ..ProjectionBatch::default()
        };
        let mut session = if let Some(session) = self.sessions.remove(&key) {
            session
        } else {
            if self.sessions.len() >= self.max_sessions {
                return output;
            }
            Http1Session::new(
                segment.direction,
                self.max_bytes,
                self.max_ranges,
                self.max_decoded_bytes,
            )
        };
        let mut next = self.next_message;
        let mut touched = session
            .active_message_id()
            .map(str::to_owned)
            .into_iter()
            .collect::<Vec<_>>();
        let mut content_failed = false;
        let mut has_body = false;
        let events = session.advance(segment, &mut next, |update| {
            if let Http1Update::Head(head) = &update {
                // A response can wait for its request before Content is emitted.
                has_body |= head.declared_body_len != Some(0);
            }
            if let Http1Update::Content { message, .. } = &update {
                content_failed |= message.content_failure.is_some();
                // Summary retention does not prove body preservation. Unknown-length
                // and unfinished bodies remain fail-open until semantic consumption.
                has_body |= message.body.len() != 0 || message.declared_body_len != Some(0);
            }
            let id = match &update {
                Http1Update::Head(head) | Http1Update::Content { head, .. } => &head.message_id,
            };
            touched.push(id.clone());
            self.consume(update, config, codecs, &mut output)
        });
        if let Some(id) = session.active_message_id() {
            touched.push(id.to_owned());
        }
        let fully_framed = !content_failed
            && !session.has_unclassified_tail()
            && !events
                .iter()
                .any(|event| matches!(event, Http1MessageEvent::Abort { .. }));
        touched.sort_unstable();
        touched.dedup();
        self.next_message = next;
        self.sessions.insert(key, session);
        self.observe_events(events, &mut output);
        self.resume_bound(config, codecs, &mut output);
        let mut output = projection.changed_actions(config, output);
        self.project_trajectory(config, projection, &mut output);
        output.payload_consumed_by_llm = fully_framed
            && !touched.is_empty()
            && touched.iter().all(|id| {
                self.facts
                    .messages
                    .get(id)
                    .is_some_and(|fact| fact.semantic.is_some())
            });
        output.payload_consumed_by_http = fully_framed
            && (!has_body || output.payload_consumed_by_llm)
            && !touched.is_empty()
            && self.retain_http
            && touched
                .iter()
                .all(|id| self.facts.messages.contains_key(id));
        let incomplete = match segment.operation_capture_state() {
            Some(model_core::payload::PayloadCaptureState::PolicyLimited) => {
                Some(Http1AbortReason::CapturePolicyLimited)
            }
            Some(model_core::payload::PayloadCaptureState::Incomplete) => {
                Some(Http1AbortReason::CaptureIncomplete)
            }
            _ => None,
        };
        if let Some(reason) = incomplete {
            output.payload_consumed_by_http = false;
            output.payload_consumed_by_llm = false;
            output.extend(self.abort_identity(
                &PayloadStreamIdentity::from_segment(segment),
                reason,
                config,
                codecs,
                projection,
            ));
        }
        self.filter_retention(output)
    }

    fn consume(
        &mut self,
        update: Http1Update<'_>,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        output: &mut ProjectionBatch,
    ) -> Option<String> {
        match update {
            Http1Update::Head(head) => {
                let http = self.facts.head(head, output);
                let observation = self.exchange.observe_http_message(
                    http.clone(),
                    &self.facts.messages[&head.message_id].identity,
                );
                output.actions.push(http);
                let mut method = None;
                for exchange in observation.matches {
                    if let Some(request_method) = &exchange.request.method {
                        if exchange.response.action_id == head.message_id {
                            method = Some(request_method.clone());
                        } else {
                            self.resumed.push((
                                exchange.response.action_id.clone(),
                                request_method.clone(),
                            ));
                        }
                    }
                    self.facts.bind(&exchange, output);
                }
                method
            }
            Http1Update::Content {
                head,
                message,
                wire,
                segments,
                evidence,
                message_start,
                force_terminal,
            } => {
                if self
                    .facts
                    .messages
                    .get(&head.message_id)
                    .is_none_or(|fact| fact.semantic_finished)
                {
                    self.sse.remove(&head.message_id);
                    return None;
                }
                let content_failure = message.content_failure.map(|failure| {
                    use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;
                    match failure {
                        crate::llm_pipeline::transport::http1::Http1DecodeFailure::BufferCapacity => StreamFinalizationReason::BufferBytesExceeded,
                        _ => StreamFinalizationReason::ProtocolDecodeFailed,
                    }
                });
                let force_terminal = force_terminal || content_failure.is_some();
                let complete = message.complete && content_failure.is_none();
                if !config.llm_layer_enabled() {
                    return None;
                }
                if head.method.is_none()
                    && !complete
                    && !force_terminal
                    && self
                        .sse
                        .get(&head.message_id)
                        .and_then(Option::as_ref)
                        .is_some_and(IncrementalSseCache::is_terminal)
                {
                    return None;
                }
                let key = PayloadStreamGroupKey {
                    trace_id: head.source.trace_id,
                    process: head.source.process,
                    stream_key: head.source.stream_key.clone(),
                    source_boundary: head.source.source_boundary,
                };
                let projected = if head.method.is_some() {
                    if content_failure.is_some() {
                        None
                    } else {
                        if !complete && !force_terminal {
                            return None;
                        }
                        project_decoded_http1_request(
                            config,
                            codecs,
                            &key,
                            message_start,
                            wire,
                            message,
                            &segments,
                        )
                    }
                } else {
                    project_decoded_http1_response(
                        config,
                        codecs,
                        &key,
                        message_start,
                        wire,
                        message,
                        evidence,
                        self.sse.entry(head.message_id.clone()).or_default(),
                        complete || force_terminal,
                    )
                };
                if let Some(fact) = self.facts.messages.get(&head.message_id) {
                    output.diagnostics.extend(
                        codecs.take_decode_diagnostic(
                            &fact.identity,
                            evidence
                                .last
                                .as_ref()
                                .map_or(head.source.observed_at, |segment| segment.observed_at),
                        ),
                    );
                }
                if let Some(mut projected) = projected {
                    if head.method.is_none() && !complete {
                        if !force_terminal {
                            // Keep provider completion in the cache until HTTP framing
                            // and content decoding have both completed successfully.
                            return None;
                        }
                        // The lifecycle event or content failure owns finalization.
                        projected.terminal = false;
                    }
                    if projected.terminal {
                        self.sse.remove(&head.message_id);
                    }
                    if let Some(fact) = self.facts.messages.get_mut(&head.message_id) {
                        fact.provider_response_id = projected
                            .provider_response_ids
                            .first()
                            .map(|metadata| metadata.provider_response_id.clone());
                    }
                    if config.l0_llm_call.context_compact.enabled
                        && let Some(fact) = self.facts.messages.get_mut(&head.message_id)
                    {
                        fact.response_observation = projected.response_observations.pop();
                    }
                    output
                        .request_observations
                        .extend(projected.request_observations);
                    for action in &mut projected.actions {
                        if !complete && force_terminal {
                            action.status = semantic_action::SemanticActionStatus::Error;
                            action.completeness =
                                semantic_action::SemanticActionCompleteness::Partial;
                        }
                        action
                            .attributes
                            .insert("http.message_id".into(), head.message_id.clone());
                        self.facts.semantic(
                            &head.message_id,
                            action.clone(),
                            projected.terminal,
                            output,
                        );
                    }
                    output.actions.extend(projected.actions);
                    output
                        .llm_request_contents
                        .extend(projected.llm_request_contents);
                    output
                        .llm_request_histories
                        .extend(projected.llm_request_histories);
                    output.llm_tool_results.extend(projected.llm_tool_results);
                    output.payload_segments.extend(projected.payload_segments);
                    output
                        .provider_response_ids
                        .extend(projected.provider_response_ids);
                }
                if let Some(reason) = content_failure {
                    self.sse.remove(&head.message_id);
                    self.facts.finalize_content(
                        &head.message_id,
                        reason,
                        evidence
                            .last
                            .as_ref()
                            .map_or(head.source.observed_at, |segment| segment.observed_at),
                        output,
                    );
                }
                None
            }
        }
    }

    fn project_trajectory(
        &mut self,
        config: &SemanticRetentionConfig,
        projection: &mut ProjectionCoordinator,
        output: &mut ProjectionBatch,
    ) {
        for binding in self
            .facts
            .take_response_bindings(config.l0_llm_call.context_compact.enabled)
        {
            output.extend(projection.observe_response_binding(binding));
        }
    }

    fn observe_events(&mut self, events: Vec<Http1MessageEvent>, output: &mut ProjectionBatch) {
        for event in &events {
            self.facts.lifecycle(event, output);
            let id = match event {
                Http1MessageEvent::End(end) => Some(&end.message_id),
                Http1MessageEvent::Abort { message_id, .. } => Some(message_id),
                _ => None,
            };
            if let Some(id) = id {
                self.sse.remove(id);
            }
        }
        output.http1_messages.extend(events);
    }

    fn resume_bound(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        output: &mut ProjectionBatch,
    ) {
        for (response_id, method) in std::mem::take(&mut self.resumed) {
            let Some(fact) = self.facts.messages.get(&response_id) else {
                continue;
            };
            let key = LiveStreamKey {
                group: PayloadStreamGroupKey::from_identity(&fact.identity),
                direction: LiveStreamDirection::Inbound,
            };
            let Some(mut session) = self.sessions.remove(&key) else {
                continue;
            };
            if session.bind_request(&response_id, &method) {
                let mut next = self.next_message;
                let events = session.resume(&mut next, None, |update| {
                    self.consume(update, config, codecs, output)
                });
                self.next_message = next;
                self.observe_events(events, output);
            }
            self.sessions.insert(key, session);
        }
    }

    pub(in crate::llm_pipeline) fn abort_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
        reason: Http1AbortReason,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch {
            http1_handled: true,
            ..ProjectionBatch::default()
        };
        for direction in [LiveStreamDirection::Outbound, LiveStreamDirection::Inbound] {
            let key = LiveStreamKey {
                group: PayloadStreamGroupKey::from_identity(identity),
                direction,
            };
            if let Some(mut session) = self.sessions.remove(&key) {
                let mut next = self.next_message;
                let events = session.finalize(&mut next, |update| {
                    self.consume(update, config, codecs, &mut output)
                });
                self.next_message = next;
                self.observe_events(events, &mut output);
                self.observe_events(session.abort(reason), &mut output);
            }
        }
        self.exchange.forget_payload_stream(identity);
        let mut output = projection.changed_actions(config, output);
        self.project_trajectory(config, projection, &mut output);
        self.filter_retention(output)
    }

    pub(in crate::llm_pipeline) fn merge_http_observation(
        &mut self,
        action: &SemanticAction,
    ) -> ProjectionBatch {
        let output = self.facts.merge(action);
        self.filter_retention(output)
    }

    pub(in crate::llm_pipeline) fn finish_transaction(
        &mut self,
        _segment: &PayloadSegment,
    ) -> ProjectionBatch {
        self.facts.release_completed();
        ProjectionBatch::default()
    }

    pub(in crate::llm_pipeline) fn close_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
        at: SystemTime,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        for direction in [LiveStreamDirection::Outbound, LiveStreamDirection::Inbound] {
            let key = LiveStreamKey {
                group: PayloadStreamGroupKey::from_identity(identity),
                direction,
            };
            if let Some(mut session) = self.sessions.remove(&key) {
                let mut next = self.next_message;
                let events = session.resume(&mut next, Some(at), |update| {
                    self.consume(update, config, codecs, &mut output)
                });
                self.next_message = next;
                self.observe_events(events, &mut output);
                self.observe_events(session.abort(Http1AbortReason::PeerClosed), &mut output);
            }
        }
        self.facts.finalize_missing_responses(
            identity.trace_id,
            |fact| &fact.identity == identity,
            crate::llm_pipeline::stream::finalizer::StreamFinalizationReason::PeerClosed,
            &mut output,
        );
        self.exchange.forget_payload_stream(identity);
        let mut output = projection.changed_actions(config, output);
        self.project_trajectory(config, projection, &mut output);
        self.filter_retention(output)
    }

    pub(in crate::llm_pipeline) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        _at: SystemTime,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        let keys = self
            .sessions
            .keys()
            .filter(|key| key.group.trace_id == trace_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(mut session) = self.sessions.remove(&key) {
                let mut next = self.next_message;
                let events = session.finalize(&mut next, |update| {
                    self.consume(update, config, codecs, &mut output)
                });
                self.next_message = next;
                self.observe_events(events, &mut output);
                self.observe_events(session.abort(Http1AbortReason::TraceClosed), &mut output);
            }
        }
        self.facts.finalize_missing_responses(
            trace_id,
            |fact| fact.identity.trace_id == trace_id,
            crate::llm_pipeline::stream::finalizer::StreamFinalizationReason::TraceClosed,
            &mut output,
        );
        let mut output = projection.changed_actions(config, output);
        self.project_trajectory(config, projection, &mut output);
        self.filter_retention(output)
    }

    fn filter_retention(&self, output: ProjectionBatch) -> ProjectionBatch {
        HttpMessageFacts::filter_retention(output, self.retain_http)
    }

    pub(in crate::llm_pipeline) fn take_upgrades(
        &mut self,
        identity: &PayloadStreamIdentity,
    ) -> Vec<Http1Upgrade> {
        let mut upgrades = Vec::new();
        for direction in [LiveStreamDirection::Outbound, LiveStreamDirection::Inbound] {
            let key = LiveStreamKey {
                group: PayloadStreamGroupKey::from_identity(identity),
                direction,
            };
            if let Some(mut upgrade) = self
                .sessions
                .get_mut(&key)
                .and_then(Http1Session::take_upgrade)
            {
                upgrade.request_message_id = self
                    .facts
                    .messages
                    .get(&upgrade.head.message_id)
                    .and_then(|fact| fact.request_id.clone());
                self.sessions.remove(&key);
                upgrades.push(upgrade);
            }
        }
        upgrades
    }

    pub(in crate::llm_pipeline) fn request_message_id(&self, response_id: &str) -> Option<&str> {
        self.facts.messages.get(response_id)?.request_id.as_deref()
    }

    pub(in crate::llm_pipeline) fn forget_identity(&mut self, identity: &PayloadStreamIdentity) {
        let group = PayloadStreamGroupKey::from_identity(identity);
        for direction in [LiveStreamDirection::Outbound, LiveStreamDirection::Inbound] {
            self.sessions.remove(&LiveStreamKey {
                group: group.clone(),
                direction,
            });
        }
        self.exchange.forget_payload_stream(identity);
        let ids = self
            .facts
            .trace_messages(identity.trace_id)
            .filter(|(_, fact)| &fact.identity == identity)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.facts.remove(&id);
            self.sse.remove(&id);
        }
    }

    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        self.sessions
            .retain(|key, _| key.group.trace_id != trace_id);
        self.exchange.forget_trace(trace_id);
        let ids = self
            .facts
            .trace_messages(trace_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.facts.remove(&id);
            self.sse.remove(&id);
        }
    }
}
