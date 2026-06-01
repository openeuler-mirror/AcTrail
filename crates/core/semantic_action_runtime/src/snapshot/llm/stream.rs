//! Plaintext payload stream indexing for snapshot LLM projection.

use model_core::payload::PayloadSegment;
use model_core::process::ProcessIdentity;

pub(super) struct IndexedPayloadStream<'a> {
    pub(super) bytes: Vec<u8>,
    ranges: Vec<PayloadSegmentRange<'a>>,
}

impl<'a> IndexedPayloadStream<'a> {
    pub(super) fn from_segments(segments: &[&'a PayloadSegment]) -> Self {
        let mut bytes = Vec::new();
        let mut ranges = Vec::new();
        for segment in segments {
            let start = bytes.len();
            bytes.extend_from_slice(&segment.bytes);
            ranges.push(PayloadSegmentRange {
                segment,
                start,
                end: bytes.len(),
            });
        }
        Self { bytes, ranges }
    }

    pub(super) fn segments_for_range(&self, start: usize, end: usize) -> Vec<&'a PayloadSegment> {
        self.ranges
            .iter()
            .filter(|range| range.start < end && range.end > start)
            .map(|range| range.segment)
            .collect()
    }
}

struct PayloadSegmentRange<'a> {
    segment: &'a PayloadSegment,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct PayloadStreamGroupKey {
    pub(super) process: ProcessIdentity,
    pub(super) stream_key: String,
}

impl PayloadStreamGroupKey {
    pub(super) fn from_segment(segment: &PayloadSegment) -> Self {
        Self {
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
        }
    }
}
