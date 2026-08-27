//! WebSocket normalization outcomes consumed at the facade boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use model_core::ids::TraceId;
use model_core::payload::{PayloadSourceBoundary, PayloadStreamIdentity, PayloadStreamKey};
use model_core::process::ProcessIdentity;

use crate::llm_pipeline::transport::websocket::WebSocketLlmObservation;

use super::output::ActionBatch;
use super::pipeline::LiveLlmProjector;

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct WebSocketStreamOwner {
    trace_id: TraceId,
    process: ProcessIdentity,
    prefix: String,
}

#[derive(Default)]
pub(super) struct WebSocketStreamOwnership {
    streams: BTreeMap<WebSocketStreamOwner, BTreeSet<PayloadStreamKey>>,
}

impl WebSocketStreamOwnership {
    pub(super) fn remember(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        stream_key: &PayloadStreamKey,
    ) {
        let Some(prefix) =
            crate::llm_pipeline::transport::websocket::WebSocketLlmAdapter::exchange_stream_prefix(
                stream_key.as_str(),
            )
        else {
            return;
        };
        self.streams
            .entry(WebSocketStreamOwner {
                trace_id,
                process,
                prefix: prefix.to_string(),
            })
            .or_default()
            .insert(stream_key.clone());
    }

    pub(super) fn release_stream(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        stream_key: &PayloadStreamKey,
    ) {
        let Some(prefix) =
            crate::llm_pipeline::transport::websocket::WebSocketLlmAdapter::exchange_stream_prefix(
                stream_key.as_str(),
            )
        else {
            return;
        };
        let owner = WebSocketStreamOwner {
            trace_id,
            process,
            prefix: prefix.to_string(),
        };
        if let Some(streams) = self.streams.get_mut(&owner) {
            streams.remove(stream_key);
            if streams.is_empty() {
                self.streams.remove(&owner);
            }
        }
    }

    pub(super) fn take_prefix(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        prefix: &str,
    ) -> BTreeSet<PayloadStreamKey> {
        self.streams
            .remove(&WebSocketStreamOwner {
                trace_id,
                process,
                prefix: prefix.to_string(),
            })
            .unwrap_or_default()
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|owner, _| owner.trace_id != trace_id);
    }
}

impl LiveLlmProjector {
    pub(super) fn consume_websocket_observation(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        source_boundary: PayloadSourceBoundary,
        observed_at: SystemTime,
        observed_stream_key: &str,
        websocket: WebSocketLlmObservation,
    ) -> ActionBatch {
        let mut changed = ActionBatch::default();
        for candidate in &websocket.projected {
            changed.extend(self.observe_http_payload(candidate));
        }
        for stream_key in &websocket.partial_exchange_streams {
            changed.extend(self.finalize_payload_stream(
                &PayloadStreamIdentity {
                    trace_id,
                    process,
                    source_boundary,
                    stream_key: stream_key.clone(),
                },
                observed_at,
            ));
        }
        self.forget_completed_websocket_exchange_streams(
            trace_id,
            &process,
            &websocket.completed_exchange_streams,
            &websocket.partial_exchange_streams,
        );
        self.forget_websocket_exchange_streams(
            trace_id,
            &process,
            &websocket.forgotten_exchange_streams,
        );
        if websocket.capacity_evicted_entries > 0 {
            let mut diagnostic = LlmPipelineDiagnostic::new(
                trace_id,
                &process,
                observed_at,
                LlmPipelineDiagnosticCode::WebSocketConnectionCapacityEvicted,
                LlmPipelineDiagnosticSeverity::Warning,
                LlmPipelineDiagnosticStage::WebSocket,
            )
            .with_stream_key(observed_stream_key)
            .with_discarded_entries(websocket.capacity_evicted_entries);
            if websocket.capacity_evicted_bytes > 0 {
                diagnostic = diagnostic.with_discarded_bytes(websocket.capacity_evicted_bytes);
            }
            changed.diagnostics.push(diagnostic);
        }
        if websocket.buffered_frame_discarded_bytes > 0 {
            changed.diagnostics.push(
                LlmPipelineDiagnostic::new(
                    trace_id,
                    &process,
                    observed_at,
                    LlmPipelineDiagnosticCode::WebSocketPendingFrameBufferExceeded,
                    LlmPipelineDiagnosticSeverity::Warning,
                    LlmPipelineDiagnosticStage::WebSocket,
                )
                .with_stream_key(observed_stream_key)
                .with_discarded_bytes(websocket.buffered_frame_discarded_bytes)
                .with_discarded_entries(1),
            );
        }
        if websocket.oversized_response_discarded_bytes > 0 {
            changed.diagnostics.push(
                LlmPipelineDiagnostic::new(
                    trace_id,
                    &process,
                    observed_at,
                    LlmPipelineDiagnosticCode::WebSocketResponseBytesExceeded,
                    LlmPipelineDiagnosticSeverity::Warning,
                    LlmPipelineDiagnosticStage::WebSocket,
                )
                .with_stream_key(observed_stream_key)
                .with_discarded_bytes(websocket.oversized_response_discarded_bytes),
            );
        }
        if websocket.decode_failed_entries > 0 {
            let mut diagnostic = LlmPipelineDiagnostic::new(
                trace_id,
                &process,
                observed_at,
                LlmPipelineDiagnosticCode::WebSocketDecodeFailed,
                LlmPipelineDiagnosticSeverity::Warning,
                LlmPipelineDiagnosticStage::WebSocket,
            )
            .with_stream_key(observed_stream_key)
            .with_discarded_entries(websocket.decode_failed_entries);
            if websocket.decode_discarded_bytes > 0 {
                diagnostic = diagnostic.with_discarded_bytes(websocket.decode_discarded_bytes);
            }
            changed.diagnostics.push(diagnostic);
        }
        if websocket.superseded_responses > 0 {
            changed.diagnostics.push(
                LlmPipelineDiagnostic::new(
                    trace_id,
                    &process,
                    observed_at,
                    LlmPipelineDiagnosticCode::WebSocketActiveResponseSuperseded,
                    LlmPipelineDiagnosticSeverity::Warning,
                    LlmPipelineDiagnosticStage::WebSocket,
                )
                .with_stream_key(observed_stream_key)
                .with_discarded_entries(websocket.superseded_responses),
            );
        }
        if websocket.lifecycle_gap_entries > 0 {
            changed.diagnostics.push(
                LlmPipelineDiagnostic::new(
                    trace_id,
                    &process,
                    observed_at,
                    LlmPipelineDiagnosticCode::WebSocketLifecycleInterrupted,
                    LlmPipelineDiagnosticSeverity::Warning,
                    LlmPipelineDiagnosticStage::WebSocket,
                )
                .with_stream_key(observed_stream_key)
                .with_discarded_entries(websocket.lifecycle_gap_entries),
            );
        }
        changed
    }
}

pub(super) fn trace_finalize_stream_key() -> &'static str {
    "websocket:trace_finalize"
}
