//! Live LLM projection from retained plaintext payload segments.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadSegment, PayloadSourceBoundary, PayloadStreamIdentity, PayloadStreamKey,
};
use model_core::process::ProcessIdentity;

use super::super::assembly::router::{
    AssemblyLimits, AssemblyResetReason, LiveStreamDirection, LiveStreamKey, LiveStreamState,
    PayloadStreamGroupKey, plaintext_http_candidate,
};
use super::super::provider::codec::LlmCodecRegistry;
use super::super::stream::finalizer::{ResponseFinalizer, StreamFinalizationReason};

use super::super::projection::ProjectionCoordinator;
use super::super::projection::correlation::{self as call, LlmStreamKey};
use super::super::transport::websocket;
use super::output::ActionBatch as LiveLlmOutput;

pub(crate) struct LiveLlmProjector {
    pub(super) config: SemanticRetentionConfig,
    pub(super) codecs: LlmCodecRegistry,
    pub(super) streams: BTreeMap<LiveStreamKey, LiveStreamState>,
    pub(super) projection: ProjectionCoordinator,
    pub(super) assembly_limits: AssemblyLimits,
    pub(super) websocket: websocket::WebSocketLlmAdapter,
    pub(super) websocket_stream_ownership: super::websocket::WebSocketStreamOwnership,
}

impl LiveLlmProjector {
    pub(super) fn observe_payload_segment(&mut self, segment: &PayloadSegment) -> LiveLlmOutput {
        let stream_key = LlmStreamKey {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
            http_stream_id: None,
        };
        let mut localized = self.projection.take_localized_output(&stream_key);
        if !self.config.llm_layer_enabled() {
            return localized;
        }
        if !plaintext_http_candidate(segment) {
            return localized;
        }
        let websocket = self.websocket.observe(segment);
        let mut changed = self.observe_http_payload(segment);
        changed.extend(self.consume_websocket_observation(
            segment.trace_id,
            segment.process,
            segment.source_boundary,
            segment.observed_at,
            segment.stream_key.as_str(),
            websocket,
        ));
        localized.extend(changed);
        localized
    }

    pub(super) fn observe_payload_gap(&mut self, segment: &PayloadSegment) -> LiveLlmOutput {
        if !self.config.llm_layer_enabled() || !plaintext_http_candidate(segment) {
            return LiveLlmOutput::default();
        }
        let key = LiveStreamKey::from_segment(segment);
        let output = self
            .streams
            .entry(key.clone())
            .or_default()
            .reset_for_discontinuity(
                &self.config,
                &self.codecs,
                &key,
                segment,
                AssemblyResetReason::ConfirmedGap,
                usize::try_from(segment.original_size).unwrap_or(usize::MAX),
            );
        let identity = PayloadStreamIdentity::from_segment(segment);
        let mut changed = self.projection.changed_actions(&self.config, output);
        changed.extend(self.finalize_missing_response_calls(
            &identity,
            StreamFinalizationReason::ConfirmedGap,
            segment.observed_at,
        ));
        self.forget_payload_associations(segment);
        changed
    }

    pub(super) fn forget_payload_associations(&mut self, segment: &PayloadSegment) {
        self.forget_payload_associations_by_identity(&PayloadStreamIdentity::from_segment(segment));
    }

    pub(super) fn finalize_payload_stream(
        &mut self,
        identity: &PayloadStreamIdentity,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let stream_key = identity.stream_key.to_string();
        let mut projected = LiveLlmOutput::default();
        for direction in [LiveStreamDirection::Outbound, LiveStreamDirection::Inbound] {
            let key = LiveStreamKey {
                group: PayloadStreamGroupKey {
                    trace_id: identity.trace_id,
                    process: identity.process,
                    stream_key: stream_key.clone(),
                },
                direction,
            };
            let Some(mut state) = self.streams.remove(&key) else {
                continue;
            };
            projected.extend(state.materialize_closed(
                &self.config,
                &self.codecs,
                &key,
                StreamFinalizationReason::PeerClosed,
                finished_at,
            ));
        }
        let mut output = self.projection.changed_actions(&self.config, projected);
        output.extend(self.finalize_missing_response_calls(
            identity,
            StreamFinalizationReason::PeerClosed,
            finished_at,
        ));
        self.forget_payload_associations_by_identity(identity);
        self.websocket_stream_ownership.release_stream(
            identity.trace_id,
            identity.process,
            &identity.stream_key,
        );
        output
    }

    fn finalize_missing_response_calls(
        &mut self,
        identity: &PayloadStreamIdentity,
        reason: StreamFinalizationReason,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let requests = self.projection.open_requests_for_identity(identity);
        let mut missing_responses = LiveLlmOutput::default();
        for request in requests {
            let mut call = call::llm_call_from_request_response(&request, None);
            ResponseFinalizer::finalize_partial(&mut call, reason, finished_at);
            missing_responses.actions.push(call);
        }
        self.projection
            .changed_actions(&self.config, missing_responses)
    }

    pub(super) fn forget_payload_stream(&mut self, identity: &PayloadStreamIdentity) {
        let stream_key = identity.stream_key.to_string();
        for direction in [LiveStreamDirection::Outbound, LiveStreamDirection::Inbound] {
            self.streams.remove(&LiveStreamKey {
                group: PayloadStreamGroupKey {
                    trace_id: identity.trace_id,
                    process: identity.process,
                    stream_key: stream_key.clone(),
                },
                direction,
            });
        }
        self.forget_payload_associations_by_identity(identity);
        self.websocket_stream_ownership.release_stream(
            identity.trace_id,
            identity.process,
            &identity.stream_key,
        );
    }

    fn forget_payload_associations_by_identity(&mut self, identity: &PayloadStreamIdentity) {
        self.projection.forget_identity(identity);
    }

    pub(super) fn forget_websocket_exchange_streams(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        prefixes: &[websocket::WebSocketExchangeStreamPrefix],
    ) {
        if prefixes.is_empty() {
            return;
        }
        for prefix in prefixes {
            let streams =
                self.websocket_stream_ownership
                    .take_prefix(trace_id, *process, prefix.as_str());
            for stream_key in streams {
                self.forget_payload_stream(&PayloadStreamIdentity {
                    trace_id,
                    process: *process,
                    source_boundary: PayloadSourceBoundary::TlsUserSpace,
                    stream_key,
                });
            }
        }
    }

    pub(super) fn forget_completed_websocket_exchange_streams(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        completed: &[PayloadStreamKey],
        partial: &[PayloadStreamKey],
    ) {
        if completed.is_empty() && partial.is_empty() {
            return;
        }
        for stream_key in completed.iter().chain(partial) {
            self.forget_payload_stream(&PayloadStreamIdentity {
                trace_id,
                process: *process,
                source_boundary: PayloadSourceBoundary::TlsUserSpace,
                stream_key: stream_key.clone(),
            });
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.websocket.forget_trace(trace_id);
        self.websocket_stream_ownership.forget_trace(trace_id);
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        self.projection.forget_trace_state(trace_id);
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        for finalization in self.websocket.finalize_trace(trace_id, finished_at) {
            output.extend(self.consume_websocket_observation(
                trace_id,
                finalization.process,
                PayloadSourceBoundary::TlsUserSpace,
                finished_at,
                super::websocket::trace_finalize_stream_key(),
                finalization.observation,
            ));
        }
        let keys = self
            .streams
            .keys()
            .filter(|key| key.group.trace_id == trace_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            let Some(mut state) = self.streams.remove(&key) else {
                continue;
            };
            let projected = state.materialize_closed(
                &self.config,
                &self.codecs,
                &key,
                StreamFinalizationReason::TraceClosed,
                finished_at,
            );
            output.extend(self.projection.changed_actions(&self.config, projected));
        }
        self.projection.finalize_trajectory(trace_id, &mut output);
        // Pair residual open requests with pending responses on streams that
        // had no confirmed (non-CONNECT) HTTP exchange (pure TLS tunnels),
        // before emitting "no response" error calls for the remaining requests.
        output.extend(
            self.projection
                .reconcile_unconfirmed_stream_exchanges(trace_id, finished_at),
        );
        for request in self.projection.open_requests_for_trace(trace_id) {
            let mut call = call::llm_call_from_request_response(&request, None);
            ResponseFinalizer::finalize_partial(
                &mut call,
                StreamFinalizationReason::TraceClosed,
                finished_at,
            );
            output.actions.push(call);
        }
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        self.websocket_stream_ownership.forget_trace(trace_id);
        self.projection.forget_trace_state(trace_id);
        output
    }
}
