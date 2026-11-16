// Tests for the corresponding production module.
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use model_core::payload::{
    PayloadContentState, PayloadOperationCompletionState, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessObservation;

use super::*;

impl PayloadSegmentReorderer {
    fn pending_segment_count(&self) -> usize {
        self.streams
            .values()
            .map(|state| state.pending.values().map(VecDeque::len).sum::<usize>())
            .sum()
    }
}

fn segment(
    sequence: u64,
    observed_at: SystemTime,
    stream_key: &str,
    direction: PayloadDirection,
) -> RawPayloadSegment {
    RawPayloadSegment {
        trace_id: TraceId::new(1),
        observed_at,
        process: ProcessObservation::host(model_core::process::HostProcessCoordinates::new(42, 0)),
        source_boundary: PayloadSourceBoundary::Syscall,
        content_state: PayloadContentState::Plaintext,
        direction,
        stream_key: PayloadStreamKey::new(stream_key.to_string()),
        sequence,
        original_size: 1,
        captured_size: 1,
        operation_id: sequence,
        operation_chunk_index: 0,
        operation_offset: 0,
        operation_original_size: 1,
        operation_captured_size: 1,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        library: "test".to_string(),
        symbol: "test".to_string(),
        protocol_hint: None,
        bytes: vec![b'x'],
    }
}

/// Matches the `now` used by the tests below so a segment is fresh
/// (within the reorder window) rather than already expired.
fn fresh(sequence: u64) -> RawPayloadSegment {
    segment(
        sequence,
        UNIX_EPOCH + Duration::from_secs(1_000_000),
        "socket:42:3:0",
        PayloadDirection::Inbound,
    )
}

#[test]
fn in_order_segments_emit_without_delay() {
    let mut reorderer = PayloadSegmentReorderer::default();
    let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let first = reorderer.admit(now, vec![fresh(1)]);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].segment.sequence, 1);
    let second = reorderer.admit(now, vec![fresh(2)]);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].segment.sequence, 2);
    assert_eq!(reorderer.pending_segment_count(), 0);
}

#[test]
fn out_of_order_chunks_are_held_and_emitted_in_capture_order() {
    let mut reorderer = PayloadSegmentReorderer::default();
    let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
    // Chunk 2 arrives first; it must be held, not emitted out of order.
    assert!(reorderer.admit(now, vec![fresh(2)]).is_empty());
    assert_eq!(reorderer.pending_segment_count(), 1);
    // Chunk 1 arrives one window later; both now emit in capture order.
    let ready = reorderer.admit(now + REORDER_WINDOW, vec![fresh(1)]);
    let sequences = ready
        .iter()
        .map(|segment| segment.segment.sequence)
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2]);
    assert_eq!(reorderer.pending_segment_count(), 0);
}

#[test]
fn gap_in_sequence_emits_after_window_elapses() {
    let mut reorderer = PayloadSegmentReorderer::default();
    let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
    // Chunk 3 arrives alone; sequences 1 and 2 are permanently missing.
    assert!(reorderer.admit(now, vec![fresh(3)]).is_empty());
    // After the reorder window elapses the gap is concluded lost.
    let ready = reorderer.admit(now + REORDER_WINDOW * 2, Vec::new());
    let sequences = ready
        .iter()
        .map(|segment| segment.segment.sequence)
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![3]);
    assert_eq!(reorderer.pending_segment_count(), 0);
}

#[test]
fn empty_admit_flushes_expired_pending() {
    let mut reorderer = PayloadSegmentReorderer::default();
    let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
    assert!(reorderer.admit(now, vec![fresh(5)]).is_empty());
    // A later admit (even with no new segments) flushes the expired chunk.
    let ready = reorderer.admit(now + REORDER_WINDOW * 2, Vec::new());
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].segment.sequence, 5);
}

#[test]
fn same_sequence_chunks_keep_arrival_order() {
    let mut reorderer = PayloadSegmentReorderer::default();
    let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let mut first = fresh(1);
    first.bytes = vec![b'a'];
    let mut second = fresh(1);
    second.bytes = vec![b'b'];
    let ready = reorderer.admit(now, vec![first, second]);
    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0].segment.bytes, vec![b'a']);
    assert_eq!(ready[1].segment.bytes, vec![b'b']);
}
