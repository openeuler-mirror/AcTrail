//! Native WebSocket transport and semantic lifecycle meet at one boundary.
use super::output::ActionBatch;
use super::pipeline::LiveLlmProjector;
use crate::llm_pipeline::transport::websocket::WebSocketAbortReason;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use std::time::SystemTime;
impl LiveLlmProjector {
    pub(super) fn observe_websocket_frames(&mut self, segment: &PayloadSegment) -> ActionBatch {
        let observed = self.websocket.observe(segment);
        let mut output =
            self.websocket_protocol
                .observe(observed.events, &self.config, &mut self.projection);
        if let Some(reason) = observed.stopped {
            output.extend(self.websocket_protocol.abort_identity(
                &PayloadStreamIdentity::from_segment(segment),
                reason,
                &self.config,
                &mut self.projection,
            ));
        }
        output
    }
    pub(super) fn abort_websocket_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
        reason: WebSocketAbortReason,
        at: SystemTime,
    ) -> ActionBatch {
        let events = self.websocket.abort_identity(identity, reason, at);
        let mut output =
            self.websocket_protocol
                .observe(events, &self.config, &mut self.projection);
        output.extend(self.websocket_protocol.abort_identity(
            identity,
            reason,
            &self.config,
            &mut self.projection,
        ));
        output
    }
}
