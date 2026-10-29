//! HTTP/2 shutdown uses each message's last captured time.
use super::protocol::Http2Protocol;
use super::state::ConnectionKey;
use crate::llm_pipeline::assembly::router::{LiveStreamDirection, PayloadStreamGroupKey};
use crate::llm_pipeline::projection::{ProjectionBatch, ProjectionCoordinator};
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;
use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::PayloadStreamIdentity;
use std::time::SystemTime;

impl Http2Protocol {
    pub(super) fn release_streams(&mut self, trace_id: TraceId, released: usize) {
        if let Some(count) = self.stream_counts.get_mut(&trace_id) {
            *count = count.saturating_sub(released);
            if *count == 0 {
                self.stream_counts.remove(&trace_id);
            }
        }
    }
    pub(in crate::llm_pipeline) fn abort_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
        reason: StreamFinalizationReason,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        let Some(mut connection) = self.connections.remove(&ConnectionKey(identity.clone())) else {
            return output;
        };
        output.http2_handled = true;
        self.release_streams(identity.trace_id, connection.streams.len());
        let group = PayloadStreamGroupKey::from_identity(identity);
        Self::diagnose(reason, &group, connection.observed_at, None, &mut output);
        let ids = connection.streams.keys().copied().collect::<Vec<_>>();
        for stream_id in ids {
            for direction in [LiveStreamDirection::Outbound, LiveStreamDirection::Inbound] {
                let id = connection.message_id(stream_id, direction);
                let Some(fact) = self.facts.messages.get(&id) else {
                    continue;
                };
                let observed_at = fact.last_observed;
                let ended = fact.ended;
                if config.llm_layer_enabled() {
                    if let Some(state) = connection.directions.get_mut(&direction) {
                        let projected = state.body.abort_stream(
                            stream_id,
                            reason,
                            observed_at,
                            direction,
                            config,
                            codecs,
                            &group,
                        );
                        self.consume(
                            projected,
                            &mut output,
                            config.l0_llm_call.context_compact.enabled,
                        );
                    }
                }
                if ended {
                    continue;
                }
                let abort = match reason {
                    StreamFinalizationReason::CapturePolicyLimited => {
                        crate::Http1AbortReason::CapturePolicyLimited
                    }
                    StreamFinalizationReason::TraceClosed => crate::Http1AbortReason::TraceClosed,
                    StreamFinalizationReason::PeerClosed => crate::Http1AbortReason::PeerClosed,
                    StreamFinalizationReason::ConfirmedGap => crate::Http1AbortReason::CaptureGap,
                    StreamFinalizationReason::BufferBytesExceeded
                    | StreamFinalizationReason::SegmentRangesExceeded
                    | StreamFinalizationReason::Http2ActiveStreamCapacity => {
                        crate::Http1AbortReason::CapacityExceeded
                    }
                    StreamFinalizationReason::ProtocolDecodeFailed => {
                        crate::Http1AbortReason::DecodeFailure
                    }
                    _ => crate::Http1AbortReason::CaptureIncomplete,
                };
                self.facts.lifecycle(
                    &crate::Http1MessageEvent::Abort {
                        message_id: id,
                        reason: abort,
                        observed_at,
                    },
                    &mut output,
                );
            }
        }
        let prefix = format!("{}:stream:", connection.prefix);
        self.facts.finalize_missing_responses(
            identity.trace_id,
            |fact| fact.http.action_id.starts_with(&prefix),
            reason,
            &mut output,
        );
        output
            .diagnostics
            .extend(codecs.take_decode_diagnostic(identity, connection.observed_at));
        self.finish_output(output, config, projection)
    }
    pub(in crate::llm_pipeline) fn close_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
        _at: SystemTime,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        self.abort_identity(
            identity,
            StreamFinalizationReason::PeerClosed,
            config,
            codecs,
            projection,
        )
    }
    pub(in crate::llm_pipeline) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        _at: SystemTime,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        projection: &mut ProjectionCoordinator,
    ) -> ProjectionBatch {
        let identities = self
            .connections
            .keys()
            .filter(|key| key.0.trace_id == trace_id)
            .map(|key| key.0.clone())
            .collect::<Vec<_>>();
        let mut output = ProjectionBatch::default();
        for identity in identities {
            output.extend(self.abort_identity(
                &identity,
                StreamFinalizationReason::TraceClosed,
                config,
                codecs,
                projection,
            ));
        }
        output
    }
    pub(in crate::llm_pipeline) fn forget_identity(&mut self, identity: &PayloadStreamIdentity) {
        if let Some(connection) = self.connections.remove(&ConnectionKey(identity.clone())) {
            self.release_streams(identity.trace_id, connection.streams.len());
        }
        let ids = self
            .facts
            .trace_messages(identity.trace_id)
            .filter(|(_, fact)| &fact.identity == identity)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.facts.remove(&id);
        }
    }
    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        self.connections.retain(|key, _| key.0.trace_id != trace_id);
        self.stream_counts.remove(&trace_id);
        let ids = self
            .facts
            .trace_messages(trace_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.facts.remove(&id);
        }
    }
}
