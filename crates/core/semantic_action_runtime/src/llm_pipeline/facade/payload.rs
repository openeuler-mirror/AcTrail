//! Incremental protocol routing before any semantic interpretation.
use super::output::ActionBatch;
use super::pipeline::LiveLlmProjector;
use super::router::ProtocolRoute;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
impl LiveLlmProjector {
    pub(super) fn observe_routed_payload(&mut self, segment: &PayloadSegment) -> ActionBatch {
        let (route, buffered) = self.protocol_router.route(segment);
        let mut output = ActionBatch::default();
        match route {
            ProtocolRoute::Pending | ProtocolRoute::Discard => output.http1_handled = true,
            ProtocolRoute::DiscardHttp2 => output.http2_handled = true,
            ProtocolRoute::Http1 => {
                for prior in &buffered {
                    output.extend(self.observe_shared_http1(prior));
                }
                let current = self.observe_shared_http1(segment);
                output.payload_consumed_by_http = current.payload_consumed_by_http;
                output.payload_consumed_by_llm = current.payload_consumed_by_llm;
                output.extend(current);
            }
            ProtocolRoute::Http2 => {
                let identity = PayloadStreamIdentity::from_segment(segment);
                for prior in &buffered {
                    output.extend(self.observe_shared_http2(prior));
                    if !self.http2.has_identity(&identity) {
                        return output;
                    }
                }
                let current = self.observe_shared_http2(segment);
                output.payload_consumed_by_http = current.payload_consumed_by_http;
                output.payload_consumed_by_llm = current.payload_consumed_by_llm;
                output.extend(current);
            }
            ProtocolRoute::WebSocket => {
                output.http1_handled = true;
                output.extend(self.observe_websocket_frames(segment));
            }
            ProtocolRoute::Legacy => {}
        };
        output
    }
    fn observe_shared_http2(&mut self, segment: &PayloadSegment) -> ActionBatch {
        let output = self
            .http2
            .advance(segment, &self.config, &self.codecs, &mut self.projection);
        let identity = PayloadStreamIdentity::from_segment(segment);
        if !self.http2.has_identity(&identity)
            || segment
                .operation_capture_state()
                .is_some_and(|state| state != model_core::payload::PayloadCaptureState::Complete)
        {
            self.protocol_router.invalidate_identity(&identity);
        }
        output
    }
    fn observe_shared_http1(&mut self, segment: &PayloadSegment) -> ActionBatch {
        let mut output =
            self.http1
                .advance(segment, &self.config, &self.codecs, &mut self.projection);
        let websocket_enabled = self.config.projection_enabled
            && (self.config.l2_websocket.enabled || self.config.llm_layer_enabled());
        if websocket_enabled {
            for event in &output.http1_messages {
                if let crate::Http1MessageEvent::Head(head) = event {
                    self.websocket.observe_head(head);
                    self.websocket
                        .observe_response(head, self.http1.request_message_id(&head.message_id));
                }
            }
        }
        let identity = PayloadStreamIdentity::from_segment(segment);
        for upgrade in self.http1.take_upgrades(&identity) {
            output.payload_consumed_by_http = false;
            output.payload_consumed_by_llm = false;
            let upgraded_identity = PayloadStreamIdentity {
                trace_id: upgrade.head.source.trace_id,
                process: upgrade.head.source.process,
                source_boundary: upgrade.head.source.source_boundary,
                stream_key: model_core::payload::PayloadStreamKey::new(
                    &upgrade.head.source.stream_key,
                ),
            };
            let ws = upgrade.head.status_code.as_deref() == Some("101")
                && upgrade
                    .head
                    .headers_text
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .any(|(name, value)| {
                        name.trim().eq_ignore_ascii_case("upgrade")
                            && value.trim().eq_ignore_ascii_case("websocket")
                    });
            if ws {
                self.protocol_router.upgrade(&upgraded_identity);
                if websocket_enabled
                    && self.websocket.upgrade(
                        &upgrade.head,
                        upgrade.request_message_id.as_deref(),
                        &self.config,
                    )
                {
                    for tail in &upgrade.tails {
                        if !tail.bytes.is_empty() {
                            output.extend(self.observe_websocket_frames(tail));
                        }
                    }
                }
            } else {
                self.protocol_router.forget_identity(&upgraded_identity);
                for tail in upgrade.tails {
                    if !tail.bytes.is_empty() {
                        output.extend(self.observe_routed_payload(&tail));
                    }
                }
            }
        }
        output
    }
}
