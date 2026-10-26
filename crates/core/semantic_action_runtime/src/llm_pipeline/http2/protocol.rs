//! One frame owner establishes HTTP/2 message sources before provider parsing.
use super::state::{Connection, ConnectionKey};
use crate::llm_pipeline::assembly::router::{
    AssemblyLimits, LiveStreamDirection, PayloadStreamGroupKey,
};
use crate::llm_pipeline::http::HttpMessageFacts;
use crate::llm_pipeline::projection::{ProjectionBatch, ProjectionCoordinator};
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;
use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use semantic_action::SemanticAction;
use std::collections::BTreeMap;

pub(in crate::llm_pipeline) struct Http2Protocol {
    pub(super) connections: BTreeMap<ConnectionKey, Connection>,
    pub(super) facts: HttpMessageFacts,
    next_connection: u64,
    pub(super) limits: AssemblyLimits,
    pub(super) stream_counts: BTreeMap<TraceId, usize>,
    pub(super) max_streams: usize,
    pub(super) retain_http: bool,
    max_frame_bytes: usize,
    max_connection_bytes: usize,
    max_evidence_ranges: usize,
}
impl Http2Protocol {
    pub(in crate::llm_pipeline) fn new(config: &SemanticRetentionConfig) -> Self {
        Self {
            connections: BTreeMap::new(),
            facts: HttpMessageFacts::new(
                config.l2_http.exchange.max_message_facts_per_trace as usize,
            ),
            next_connection: 0,
            limits: config.l0_llm_call.assembly.into(),
            stream_counts: BTreeMap::new(),
            max_streams: config.l2_http.http2.max_active_streams_per_trace as usize,
            retain_http: config.http_message_summary_enabled(),
            max_frame_bytes: config.l2_http.http2.max_frame_bytes as usize,
            max_connection_bytes: config.l2_http.http2.max_connection_buffer_bytes as usize,
            max_evidence_ranges: config.l2_http.http2.max_evidence_ranges as usize,
        }
    }
    pub(in crate::llm_pipeline) fn has_identity(&self, identity: &PayloadStreamIdentity) -> bool {
        self.connections
            .contains_key(&ConnectionKey(identity.clone()))
    }
    pub(in crate::llm_pipeline) fn advance(
        &mut self,
        segment: &PayloadSegment,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let identity = PayloadStreamIdentity::from_segment(segment);
        let key = ConnectionKey(identity.clone());
        let mut output = ProjectionBatch {
            http2_handled: true,
            ..ProjectionBatch::default()
        };
        let mut connection = if let Some(connection) = self.connections.remove(&key) {
            connection
        } else {
            let Some(next) = self.next_connection.checked_add(1) else {
                tracing::error!(trace_id = %segment.trace_id, "HTTP/2 connection identity sequence exhausted");
                return output;
            };
            self.next_connection = next;
            Connection {
                observed_at: segment.observed_at,
                prefix: format!("trace:{}:h2:{}", segment.trace_id.get(), next),
                directions: BTreeMap::new(),
                streams: BTreeMap::new(),
                highest_request: 0,
                completed: Vec::new(),
            }
        };
        connection.observed_at = segment.observed_at;
        let direction = LiveStreamDirection::from(segment.direction);
        let decoder = &mut connection.direction(direction).decoder;
        let capacity_reason = if decoder
            .buffered_bytes()
            .checked_add(segment.bytes.len())
            .is_none_or(|size| size > self.max_connection_bytes)
        {
            Some(StreamFinalizationReason::BufferBytesExceeded)
        } else if decoder.evidence_ranges() >= self.max_evidence_ranges {
            Some(StreamFinalizationReason::SegmentRangesExceeded)
        } else {
            None
        };
        if let Some(reason) = capacity_reason {
            self.connections.insert(key, connection);
            return self.abort_identity(&identity, reason, config, codecs, projection);
        }
        decoder.append(segment);
        let events = decoder.advance(self.max_frame_bytes);
        let framed_all = decoder.buffered_bytes() == 0 && !segment.bytes.is_empty();
        let group = PayloadStreamGroupKey::from_segment(segment);
        let (terminal, failed, clean) = self.observe_frames(
            events,
            &mut connection,
            direction,
            &group,
            config,
            codecs,
            &mut output,
        );
        if config.llm_layer_enabled() {
            let projected = connection
                .direction(direction)
                .body
                .project(config, codecs, &group, direction);
            self.consume(projected, &mut output);
        }
        for event in terminal {
            if let crate::Http1MessageEvent::End(end) = &event {
                if self
                    .facts
                    .messages
                    .get(&end.message_id)
                    .and_then(|fact| fact.semantic.as_ref())
                    .is_some_and(|semantic| {
                        semantic.status == semantic_action::SemanticActionStatus::InProgress
                    })
                {
                    self.facts.finalize_content(
                        &end.message_id,
                        StreamFinalizationReason::ProtocolDecodeFailed,
                        end.observed_at,
                        &mut output,
                    );
                }
            }
            self.facts.lifecycle(&event, &mut output);
        }
        let all_llm = !output.http2_frames.is_empty()
            && output.http2_frames.iter().all(|frame| {
                frame
                    .message_id
                    .as_ref()
                    .and_then(|id| self.facts.messages.get(id))
                    .is_some_and(|fact| fact.semantic.is_some())
            });
        output.payload_consumed_by_llm = framed_all && clean && all_llm;
        output.payload_consumed_by_http = framed_all
            && clean
            && (all_llm
                || !output
                    .http2_frames
                    .iter()
                    .any(|frame| !frame.connection_preface && frame.frame_type == 0))
            && !output.http2_frames.is_empty()
            && output
                .http2_frames
                .iter()
                .all(|frame| frame.message_id.is_some() && self.retain_http);
        self.connections.insert(key, connection);
        let mut output = self.finish_output(output, config, projection);
        let reason = if failed.is_some() {
            failed
        } else {
            match segment.operation_capture_state() {
                Some(model_core::payload::PayloadCaptureState::PolicyLimited) => {
                    Some(StreamFinalizationReason::CapturePolicyLimited)
                }
                Some(model_core::payload::PayloadCaptureState::Incomplete) => {
                    Some(StreamFinalizationReason::OperationIncomplete)
                }
                _ => None,
            }
        };
        if let Some(reason) = reason {
            output.payload_consumed_by_http = false;
            output.payload_consumed_by_llm = false;
            output.extend(self.abort_identity(&identity, reason, config, codecs, projection));
        }
        output
            .diagnostics
            .extend(codecs.take_decode_diagnostic(&identity, segment.observed_at));
        output
    }
    pub(super) fn consume(&mut self, mut projected: ProjectionBatch, output: &mut ProjectionBatch) {
        projected.actions.retain_mut(|action| {
            let Some((id, _)) = action.action_id.rsplit_once(":llm.") else {
                return true;
            };
            let id = id.to_owned();
            let Some(fact) = self.facts.messages.get_mut(&id) else {
                return false;
            };
            fact.provider_response_id = projected
                .provider_response_ids
                .iter()
                .find(|metadata| metadata.action_id == action.action_id)
                .map(|metadata| metadata.provider_response_id.clone());
            action
                .attributes
                .insert("http.message_id".into(), id.clone());
            self.facts.semantic(
                &id,
                action.clone(),
                action.status != semantic_action::SemanticActionStatus::InProgress,
                output,
            );
            true
        });
        output.extend(projected);
    }
    pub(super) fn finish_output(
        &mut self,
        output: ProjectionBatch,
        config: &SemanticRetentionConfig,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let mut output = projection.changed_actions(config, output);
        for (request, provider_id, observed_at, failed) in self.facts.take_trajectory_bindings() {
            output.extend(projection.observe_response_trajectory(
                &request,
                provider_id.as_deref(),
                observed_at,
                failed,
            ));
        }
        HttpMessageFacts::filter_retention(output, self.retain_http)
    }
    pub(in crate::llm_pipeline) fn merge_http_observation(
        &mut self,
        action: &SemanticAction,
    ) -> ProjectionBatch {
        HttpMessageFacts::filter_retention(self.facts.merge(action), self.retain_http)
    }
    pub(in crate::llm_pipeline) fn finish_transaction(
        &mut self,
        segment: &PayloadSegment,
    ) -> ProjectionBatch {
        self.facts.release_completed();
        if let Some(connection) = self
            .connections
            .get_mut(&ConnectionKey(PayloadStreamIdentity::from_segment(segment)))
        {
            for id in std::mem::take(&mut connection.completed) {
                if connection.streams.remove(&id).is_some() {
                    if let Some(count) = self.stream_counts.get_mut(&segment.trace_id) {
                        *count = count.saturating_sub(1);
                    }
                }
            }
        }
        ProjectionBatch::default()
    }
}
