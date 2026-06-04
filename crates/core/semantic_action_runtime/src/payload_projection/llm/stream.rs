//! Plaintext payload stream indexing for LLM projection.

use model_core::ids::TraceId;
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
pub(crate) struct PayloadStreamGroupKey {
    pub(crate) trace_id: TraceId,
    pub(crate) process: ProcessIdentity,
    pub(crate) stream_key: String,
}

impl PayloadStreamGroupKey {
    pub(crate) fn from_segment(segment: &PayloadSegment) -> Self {
        Self {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct PayloadOperationKey {
    process: ProcessIdentity,
    stream_key: String,
    operation_id: u64,
    segment_id: u64,
}

impl PayloadOperationKey {
    pub(super) fn from_segment(segment: &PayloadSegment) -> Self {
        let segment_id = if segment.operation_id == 0 {
            segment.segment_id.get()
        } else {
            0
        };
        Self {
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
            operation_id: segment.operation_id,
            segment_id,
        }
    }
}
