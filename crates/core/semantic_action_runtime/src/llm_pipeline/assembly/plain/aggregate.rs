//! Wire DATA coordinates and evidence for an identified HTTP/2 message.
use crate::llm_pipeline::projection::projector::InFlightResponse;
use crate::llm_pipeline::stream::response::IncrementalSseCache;
use crate::llm_pipeline::transport::evidence::{EvidenceCursor, EvidenceSnapshot, EvidenceTracker};
use model_core::payload::PayloadSegment;
use std::sync::Arc;

#[derive(Default)]
pub(in crate::llm_pipeline) struct PlainStreamAssembly {
    pub(in crate::llm_pipeline) encoded_len: usize,
    pub(in crate::llm_pipeline) segments: EvidenceTracker,
    response_evidence: Option<EvidenceCursor>,
    pub(in crate::llm_pipeline) sse_parse_cache: Option<IncrementalSseCache>,
    pub(in crate::llm_pipeline) in_flight_response: Option<InFlightResponse>,
}
impl PlainStreamAssembly {
    pub(in crate::llm_pipeline) fn append_evidence(
        &mut self,
        encoded_len: usize,
        segments: &[PayloadSegment],
    ) {
        let start = self.encoded_len;
        self.encoded_len += encoded_len;
        let end = self.encoded_len;
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
}
