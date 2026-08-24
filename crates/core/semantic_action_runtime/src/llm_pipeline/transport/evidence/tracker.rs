//! Monotonic body-offset to payload-evidence mapping.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use model_core::payload::{
    PayloadOperationCompletionState, PayloadSegment, PayloadTruncationState,
};

#[derive(Default)]
pub(in crate::llm_pipeline) struct EvidenceTracker {
    ranges: VecDeque<EvidenceRange>,
    next_range_id: u64,
}

struct EvidenceRange {
    id: u64,
    start: usize,
    end: usize,
    segment: PayloadSegment,
}

#[derive(Default)]
pub(in crate::llm_pipeline) struct EvidenceCursor {
    message_start: usize,
    next_range_id: u64,
    snapshot: Arc<EvidenceSnapshot>,
}

#[derive(Clone, Default)]
pub(in crate::llm_pipeline) struct EvidenceSnapshot {
    pub(in crate::llm_pipeline) first: Option<PayloadSegment>,
    pub(in crate::llm_pipeline) last: Option<PayloadSegment>,
    pub(in crate::llm_pipeline) segment_count: usize,
    pub(in crate::llm_pipeline) operation_ids: BTreeSet<u64>,
    pub(in crate::llm_pipeline) any_operation_failed: bool,
    pub(in crate::llm_pipeline) all_capture_complete: bool,
    seen_segment_ids: BTreeSet<u64>,
    operation_states: BTreeMap<u64, OperationEvidenceState>,
}

#[derive(Clone, Default)]
struct OperationEvidenceState {
    expected_offset: u64,
    captured_size: u64,
    original_size: u64,
    contiguous: bool,
}

impl EvidenceCursor {
    fn new(message_start: usize, next_range_id: u64) -> Self {
        Self {
            message_start,
            next_range_id,
            snapshot: Arc::new(EvidenceSnapshot {
                all_capture_complete: true,
                ..EvidenceSnapshot::default()
            }),
            ..Self::default()
        }
    }

    pub(in crate::llm_pipeline) fn message_start(&self) -> usize {
        self.message_start
    }

    pub(in crate::llm_pipeline) fn snapshot(&self) -> Arc<EvidenceSnapshot> {
        Arc::clone(&self.snapshot)
    }
}

impl EvidenceSnapshot {
    pub(in crate::llm_pipeline) fn operation_segments_are_complete(&self) -> bool {
        self.first.is_some()
            && !self.operation_states.is_empty()
            && self.operation_states.values().all(|operation| {
                operation.contiguous
                    && operation.expected_offset == operation.captured_size
                    && operation.captured_size == operation.original_size
            })
    }

    fn observe(&mut self, segment: &PayloadSegment) {
        if !self.seen_segment_ids.insert(segment.segment_id.get()) {
            return;
        }
        let mut metadata = segment.clone();
        metadata.bytes.clear();
        if self.first.is_none() {
            self.first = Some(metadata.clone());
        }
        self.last = Some(metadata);
        self.segment_count = self.segment_count.saturating_add(1);
        self.operation_ids.insert(segment.operation_id);
        self.any_operation_failed |=
            segment.operation_completion_state == PayloadOperationCompletionState::Failed;
        self.all_capture_complete &= segment.truncation == PayloadTruncationState::Complete
            && segment.operation_completion_state == PayloadOperationCompletionState::Success
            && segment.operation_original_size == segment.operation_captured_size;
        let operation = self
            .operation_states
            .entry(segment.operation_id)
            .or_insert_with(|| OperationEvidenceState {
                captured_size: segment.operation_captured_size,
                original_size: segment.operation_original_size,
                contiguous: true,
                ..OperationEvidenceState::default()
            });
        operation.contiguous &= segment.operation_offset == operation.expected_offset
            && segment.operation_captured_size == operation.captured_size
            && segment.operation_original_size == operation.original_size;
        operation.expected_offset = operation
            .expected_offset
            .saturating_add(segment.captured_size);
    }
}

impl EvidenceTracker {
    pub(in crate::llm_pipeline) fn cursor(&self, message_start: usize) -> EvidenceCursor {
        let next_range_id = self
            .ranges
            .iter()
            .find(|range| range.end > message_start)
            .map_or(self.next_range_id, |range| range.id);
        EvidenceCursor::new(message_start, next_range_id)
    }

    pub(in crate::llm_pipeline) fn append(
        &mut self,
        start: usize,
        end: usize,
        segment: &PayloadSegment,
    ) {
        let mut metadata = segment.clone();
        metadata.bytes.clear();
        let id = self.next_range_id;
        self.next_range_id = self.next_range_id.saturating_add(1);
        self.ranges.push_back(EvidenceRange {
            id,
            start,
            end,
            segment: metadata,
        });
    }

    pub(in crate::llm_pipeline) fn advance_cursor(
        &self,
        cursor: &mut EvidenceCursor,
        end: usize,
    ) -> bool {
        if end < cursor.message_start {
            return false;
        }
        if let Some(front) = self.ranges.front()
            && cursor.next_range_id < front.id
        {
            return false;
        }
        let first = self
            .ranges
            .partition_point(|range| range.id < cursor.next_range_id);
        for range in self.ranges.iter().skip(first) {
            if range.start >= end {
                break;
            }
            cursor.next_range_id = range.id.saturating_add(1);
            if range.end > cursor.message_start {
                Arc::make_mut(&mut cursor.snapshot).observe(&range.segment);
            }
        }
        true
    }

    pub(in crate::llm_pipeline) fn for_range(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<&PayloadSegment> {
        let first = self.ranges.partition_point(|range| range.end <= start);
        self.ranges
            .iter()
            .skip(first)
            .take_while(|range| range.start < end)
            .map(|range| &range.segment)
            .collect()
    }

    pub(in crate::llm_pipeline) fn discarded_prefix_stats(&self, end: usize) -> (u64, u64) {
        let mut discarded_bytes = 0_u64;
        let mut discarded_entries = 0_u64;
        for range in self.ranges.iter().take_while(|range| range.start < end) {
            let overlap_end = range.end.min(end);
            let overlap = overlap_end.saturating_sub(range.start);
            if overlap == 0 {
                continue;
            }
            let range_bytes = range.end.saturating_sub(range.start);
            discarded_bytes = discarded_bytes.saturating_add(if overlap == range_bytes {
                range.segment.original_size
            } else {
                u64::try_from(overlap).unwrap_or(u64::MAX)
            });
            discarded_entries = discarded_entries.saturating_add(1);
        }
        (discarded_bytes, discarded_entries)
    }

    pub(in crate::llm_pipeline) fn evict_before(&mut self, global_offset: usize) {
        while self
            .ranges
            .front()
            .is_some_and(|range| range.end <= global_offset)
        {
            self.ranges.pop_front();
        }
        if let Some(front) = self.ranges.front_mut()
            && front.start < global_offset
        {
            front.start = global_offset;
        }
    }

    pub(in crate::llm_pipeline) fn len(&self) -> usize {
        self.ranges.len()
    }

    pub(in crate::llm_pipeline) fn reset(&mut self) {
        self.ranges = VecDeque::new();
    }

    pub(in crate::llm_pipeline) fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}
