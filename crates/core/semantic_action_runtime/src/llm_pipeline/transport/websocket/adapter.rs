//! HTTP-authoritative upgrade handoff; no cross-connection inference.
use super::connection::WebSocketConnection;
use super::handshake::WebSocketHandshake;
use super::model::{WebSocketAbortReason, WebSocketConnectionKey, WebSocketMessageEvent};
use crate::Http1MessageHead;
use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment, PayloadStreamIdentity};
use std::collections::BTreeMap;
use std::time::SystemTime;
struct Offer {
    request_id: String,
    path: String,
    expected_accept: String,
}
enum ConnectionState {
    Offered(Offer),
    Open(WebSocketConnection),
}
#[derive(Default)]
pub(in crate::llm_pipeline) struct WebSocketObservation {
    pub(in crate::llm_pipeline) events: Vec<WebSocketMessageEvent>,
    pub(in crate::llm_pipeline) stopped: Option<WebSocketAbortReason>,
}
pub(in crate::llm_pipeline) struct WebSocketTransport {
    connections: BTreeMap<WebSocketConnectionKey, ConnectionState>,
    max_connections: usize,
}
impl WebSocketTransport {
    pub(in crate::llm_pipeline) fn new(config: &SemanticRetentionConfig) -> Self {
        Self {
            connections: BTreeMap::new(),
            max_connections: config.l2_websocket.max_active_connections as usize,
        }
    }
    fn identity(head: &Http1MessageHead) -> PayloadStreamIdentity {
        PayloadStreamIdentity {
            trace_id: head.source.trace_id,
            process: head.source.process,
            source_boundary: head.source.source_boundary,
            stream_key: model_core::payload::PayloadStreamKey::new(&head.source.stream_key),
        }
    }
    pub(in crate::llm_pipeline) fn observe_head(&mut self, head: &Http1MessageHead) {
        if head.source.direction != PayloadDirection::Outbound {
            return;
        }
        let Some((path, expected_accept)) = WebSocketHandshake::request(head) else {
            return;
        };
        let key = WebSocketConnectionKey(Self::identity(head));
        if self.connections.contains_key(&key) {
            return;
        }
        if self.connections.len() >= self.max_connections {
            tracing::warn!(trace_id=%head.source.trace_id,stream_key=%head.source.stream_key,"WebSocket connection capacity reached");
            return;
        }
        self.connections.insert(
            key,
            ConnectionState::Offered(Offer {
                request_id: head.message_id.clone(),
                path,
                expected_accept,
            }),
        );
    }
    pub(in crate::llm_pipeline) fn upgrade(
        &mut self,
        head: &Http1MessageHead,
        request_id: Option<&str>,
        config: &SemanticRetentionConfig,
    ) -> bool {
        let key = WebSocketConnectionKey(Self::identity(head));
        let Some(ConnectionState::Offered(offer)) = self.connections.remove(&key) else {
            return false;
        };
        let Some((accept, extensions)) = WebSocketHandshake::accepted(head) else {
            return false;
        };
        if request_id != Some(offer.request_id.as_str()) || accept != offer.expected_accept {
            return false;
        }
        self.connections.insert(
            key,
            ConnectionState::Open(WebSocketConnection::new(
                format!("{}:websocket", offer.request_id),
                offer.path,
                extensions,
                config,
            )),
        );
        true
    }
    pub(in crate::llm_pipeline) fn observe_response(
        &mut self,
        head: &Http1MessageHead,
        request_id: Option<&str>,
    ) {
        let Some(status) = head
            .status_code
            .as_deref()
            .and_then(|value| value.parse::<u16>().ok())
        else {
            return;
        };
        if status < 200 {
            return;
        }
        let key = WebSocketConnectionKey(Self::identity(head));
        if self.connections.get(&key).is_some_and(|state| match state {
            ConnectionState::Offered(offer) => request_id == Some(offer.request_id.as_str()),
            ConnectionState::Open(_) => false,
        }) {
            self.connections.remove(&key);
        }
    }
    pub(in crate::llm_pipeline) fn observe(
        &mut self,
        segment: &PayloadSegment,
    ) -> WebSocketObservation {
        let identity = PayloadStreamIdentity::from_segment(segment);
        let key = WebSocketConnectionKey(identity.clone());
        let Some(ConnectionState::Open(connection)) = self.connections.get_mut(&key) else {
            return WebSocketObservation::default();
        };
        let observation = connection.observe(segment);
        let mut output = WebSocketObservation {
            events: observation.events,
            stopped: None,
        };
        let reason = match segment.operation_capture_state() {
            Some(model_core::payload::PayloadCaptureState::Incomplete) => {
                Some(WebSocketAbortReason::CaptureIncomplete)
            }
            Some(model_core::payload::PayloadCaptureState::PolicyLimited) => {
                Some(WebSocketAbortReason::CapturePolicyLimited)
            }
            _ if observation.failed => Some(
                output
                    .events
                    .iter()
                    .rev()
                    .find_map(|event| match event {
                        WebSocketMessageEvent::Abort { reason, .. } => Some(*reason),
                        _ => None,
                    })
                    .unwrap_or(WebSocketAbortReason::DecodeFailure),
            ),
            _ if observation.closed => Some(WebSocketAbortReason::PeerClosed),
            _ => None,
        };
        if let Some(reason) = reason {
            output
                .events
                .extend(self.abort_identity(&identity, reason, segment.observed_at));
            output.stopped = Some(reason);
        }
        output
    }
    pub(in crate::llm_pipeline) fn abort_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
        reason: WebSocketAbortReason,
        at: SystemTime,
    ) -> Vec<WebSocketMessageEvent> {
        match self
            .connections
            .remove(&WebSocketConnectionKey(identity.clone()))
        {
            Some(ConnectionState::Open(mut connection)) => connection.abort(reason, at),
            _ => Vec::new(),
        }
    }
    pub(in crate::llm_pipeline) fn identities(
        &self,
        trace_id: TraceId,
    ) -> Vec<PayloadStreamIdentity> {
        self.connections
            .keys()
            .filter(|key| key.0.trace_id == trace_id)
            .map(|key| key.0.clone())
            .collect()
    }
    pub(in crate::llm_pipeline) fn forget_identity(&mut self, identity: &PayloadStreamIdentity) {
        self.connections
            .remove(&WebSocketConnectionKey(identity.clone()));
    }
    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        self.connections.retain(|key, _| key.0.trace_id != trace_id);
    }
}
