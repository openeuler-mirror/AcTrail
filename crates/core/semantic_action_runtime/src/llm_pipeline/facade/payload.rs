//! Payload-segment ingress owned by the pipeline facade.

use model_core::payload::PayloadSegment;

use crate::llm_pipeline::assembly::router::LiveStreamKey;

use super::output::ActionBatch as LiveLlmOutput;
use super::pipeline::LiveLlmProjector;

impl LiveLlmProjector {
    pub(super) fn observe_http_payload(&mut self, segment: &PayloadSegment) -> LiveLlmOutput {
        let key = LiveStreamKey::from_segment(segment);
        self.websocket_stream_ownership.remember(
            segment.trace_id,
            segment.process,
            &segment.stream_key,
        );
        let output = self
            .streams
            .entry(key.clone())
            .or_default()
            .observe_segment(
                &self.config,
                &self.codecs,
                self.assembly_limits,
                &key,
                segment,
            );
        self.projection.changed_actions(&self.config, output)
    }
}
