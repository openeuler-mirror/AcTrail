//! Shared protocol ownership and native transport lifecycle.
use super::output::ActionBatch;
use crate::llm_pipeline::projection::ProjectionCoordinator;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;
use crate::llm_pipeline::transport::websocket::{WebSocketAbortReason, WebSocketTransport};
use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{PayloadCaptureState, PayloadSegment, PayloadStreamIdentity};
use std::time::SystemTime;

pub(crate) struct LiveLlmProjector {
    pub(super) http1: crate::llm_pipeline::http1::Http1Protocol,
    pub(super) http2: crate::llm_pipeline::http2::Http2Protocol,
    pub(super) websocket: WebSocketTransport,
    pub(super) websocket_protocol: crate::llm_pipeline::websocket::WebSocketProtocol,
    pub(super) protocol_router: super::router::ProtocolRouter,
    pub(super) config: SemanticRetentionConfig,
    pub(super) codecs: LlmCodecRegistry,
    pub(super) projection: ProjectionCoordinator,
}
impl LiveLlmProjector {
    pub(super) fn observe_payload_segment(&mut self, segment: &PayloadSegment) -> ActionBatch {
        if !matches!(
            segment.source_boundary,
            model_core::payload::PayloadSourceBoundary::TlsUserSpace
                | model_core::payload::PayloadSourceBoundary::Syscall
        ) || segment.content_state != model_core::payload::PayloadContentState::Plaintext
        {
            return ActionBatch::default();
        }
        self.observe_routed_payload(segment)
    }
    pub(super) fn observe_payload_gap(&mut self, segment: &PayloadSegment) -> ActionBatch {
        let identity = PayloadStreamIdentity::from_segment(segment);
        let mut output = self.http1.abort_identity(
            &identity,
            crate::Http1AbortReason::CaptureGap,
            &self.config,
            &self.codecs,
            &mut self.projection,
        );
        output.extend(self.http2.abort_identity(
            &identity,
            StreamFinalizationReason::ConfirmedGap,
            &self.config,
            &self.codecs,
            &mut self.projection,
        ));
        output.extend(self.abort_websocket_identity(
            &identity,
            WebSocketAbortReason::CaptureGap,
            segment.observed_at,
        ));
        self.protocol_router.invalidate_identity(&identity);
        output
    }
    pub(super) fn finish_incomplete_payload(&mut self, segment: &PayloadSegment) -> ActionBatch {
        let identity = PayloadStreamIdentity::from_segment(segment);
        let limited = segment.operation_capture_state() == Some(PayloadCaptureState::PolicyLimited);
        let mut output = self.http1.abort_identity(
            &identity,
            if limited {
                crate::Http1AbortReason::CapturePolicyLimited
            } else {
                crate::Http1AbortReason::CaptureIncomplete
            },
            &self.config,
            &self.codecs,
            &mut self.projection,
        );
        output.extend(self.http2.abort_identity(
            &identity,
            if limited {
                StreamFinalizationReason::CapturePolicyLimited
            } else {
                StreamFinalizationReason::OperationIncomplete
            },
            &self.config,
            &self.codecs,
            &mut self.projection,
        ));
        output.extend(self.abort_websocket_identity(
            &identity,
            if limited {
                WebSocketAbortReason::CapturePolicyLimited
            } else {
                WebSocketAbortReason::CaptureIncomplete
            },
            segment.observed_at,
        ));
        self.protocol_router.invalidate_identity(&identity);
        output
    }
    pub(super) fn finish_payload_transaction(&mut self, segment: &PayloadSegment) -> ActionBatch {
        let mut output = self.http1.finish_transaction(segment);
        output.extend(self.http2.finish_transaction(segment));
        output
    }
    pub(super) fn finalize_payload_stream(
        &mut self,
        identity: &PayloadStreamIdentity,
        at: SystemTime,
    ) -> ActionBatch {
        let mut output = self.http1.close_identity(
            identity,
            at,
            &self.config,
            &self.codecs,
            &mut self.projection,
        );
        output.extend(self.http2.close_identity(
            identity,
            at,
            &self.config,
            &self.codecs,
            &mut self.projection,
        ));
        output.extend(self.abort_websocket_identity(
            identity,
            WebSocketAbortReason::PeerClosed,
            at,
        ));
        self.protocol_router.forget_identity(identity);
        output
    }
    pub(super) fn forget_payload_stream(&mut self, identity: &PayloadStreamIdentity) {
        self.http1.forget_identity(identity);
        self.http2.forget_identity(identity);
        self.websocket.forget_identity(identity);
        self.websocket_protocol.forget_identity(identity);
        self.protocol_router.forget_identity(identity);
    }
    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.http1.forget_trace(trace_id);
        self.http2.forget_trace(trace_id);
        self.websocket.forget_trace(trace_id);
        self.websocket_protocol.forget_trace(trace_id);
        self.protocol_router.forget_trace(trace_id);
        self.projection.forget_trace_state(trace_id);
    }
    pub(super) fn finalize_trace(&mut self, trace_id: TraceId, at: SystemTime) -> ActionBatch {
        let mut output = self.http1.finalize_trace(
            trace_id,
            at,
            &self.config,
            &self.codecs,
            &mut self.projection,
        );
        output.extend(self.http2.finalize_trace(
            trace_id,
            at,
            &self.config,
            &self.codecs,
            &mut self.projection,
        ));
        for identity in self.websocket.identities(trace_id) {
            output.extend(self.abort_websocket_identity(
                &identity,
                WebSocketAbortReason::TraceClosed,
                at,
            ));
        }
        self.projection.finalize_trajectory(trace_id, &mut output);
        self.forget_trace(trace_id);
        output
    }
}
