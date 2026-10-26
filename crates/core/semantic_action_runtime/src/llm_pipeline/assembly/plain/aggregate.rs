//! Bounded semantic body bytes and evidence for an already identified HTTP/2 message.
use crate::llm_pipeline::projection::projector::InFlightResponse;
use crate::llm_pipeline::stream::response::IncrementalSseCache;
use crate::llm_pipeline::transport::buffer::CursorBuffer;
use crate::llm_pipeline::transport::evidence::{EvidenceCursor, EvidenceSnapshot, EvidenceTracker};
use model_core::payload::PayloadSegment;
use std::sync::Arc;

#[derive(Default)]
pub(in crate::llm_pipeline) struct PlainStreamAssembly {
    pub(in crate::llm_pipeline) buffer: CursorBuffer,
    pub(in crate::llm_pipeline) base_offset: usize,
    pub(in crate::llm_pipeline) segments: EvidenceTracker,
    response_evidence: Option<EvidenceCursor>,
    pub(in crate::llm_pipeline) sse_parse_cache: Option<IncrementalSseCache>,
    pub(in crate::llm_pipeline) in_flight_response: Option<InFlightResponse>,
}
impl PlainStreamAssembly {
    pub(in crate::llm_pipeline) fn append_plaintext(
        &mut self,
        bytes: &[u8],
        segments: &[PayloadSegment],
    ) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(bytes);
        let end = self.base_offset + self.buffer.len();
        for segment in segments {
            self.segments.append(start, end, segment);
        }
    }

    pub(in crate::llm_pipeline) fn segments_for_range(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<&PayloadSegment> {
        self.segments.for_range(start, end)
    }

    pub(in crate::llm_pipeline) fn response_evidence(
        &mut self,
        message_start: usize,
        message_end: usize,
    ) -> Option<Arc<EvidenceSnapshot>> {
        if self
            .response_evidence
            .as_ref()
            .is_none_or(|cursor| cursor.message_start() != message_start)
        {
            self.response_evidence = Some(self.segments.cursor(message_start));
        }
        let cursor = self.response_evidence.as_mut()?;
        self.segments
            .advance_cursor(cursor, message_end)
            .then(|| cursor.snapshot())
    }

    pub(in crate::llm_pipeline) fn evict_encoded_len(&mut self, encoded_len: usize) {
        let Some(global_end) = self.base_offset.checked_add(encoded_len) else {
            tracing::warn!(encoded_len, "refused overflowing LLM stream buffer release");
            return;
        };
        if !self.buffer.release(encoded_len) {
            tracing::warn!(
                encoded_len,
                buffered_bytes = self.buffer.len(),
                "refused out-of-range LLM stream buffer release"
            );
            return;
        }
        self.base_offset = global_end;
        self.segments.evict_before(self.base_offset);
        if self.buffer.is_empty() {
            self.segments.reset();
            self.response_evidence = None;
        }
    }
}
