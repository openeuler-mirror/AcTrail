//! A negotiated WebSocket owns its two native capture directions.
use super::framing::{DirectionAssembler, DirectionObservation};
use super::handshake::NegotiatedExtensions;
use super::model::{WebSocketAbortReason, WebSocketMessageEvent};
use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::{PayloadDirection, PayloadSegment};
use std::time::SystemTime;
pub(super) struct WebSocketConnection {
    outbound: DirectionAssembler,
    inbound: DirectionAssembler,
}
impl WebSocketConnection {
    pub(super) fn new(
        id: String,
        path: String,
        extensions: NegotiatedExtensions,
        config: &SemanticRetentionConfig,
    ) -> Self {
        Self {
            outbound: DirectionAssembler::new(
                id.clone(),
                path.clone(),
                true,
                extensions.permessage_deflate,
                extensions.client_no_context_takeover,
                config,
            ),
            inbound: DirectionAssembler::new(
                id,
                path,
                false,
                extensions.permessage_deflate,
                extensions.server_no_context_takeover,
                config,
            ),
        }
    }
    pub(super) fn observe(&mut self, segment: &PayloadSegment) -> DirectionObservation {
        match segment.direction {
            PayloadDirection::Outbound => self.outbound.push(segment),
            PayloadDirection::Inbound => self.inbound.push(segment),
        }
    }
    pub(super) fn abort(
        &mut self,
        reason: WebSocketAbortReason,
        at: SystemTime,
    ) -> Vec<WebSocketMessageEvent> {
        self.outbound
            .abort(reason, at)
            .into_iter()
            .chain(self.inbound.abort(reason, at))
            .collect()
    }
}
