//! Cross-drain reordering of kernel-captured payload segments within a stream.
//!
//! eBPF ring/perf buffers are drained per-CPU, so chunks of the same stream
//! can be handed to userspace out of causal order, straddling drain cycles.
//! Both the LLM projector and the HTTP/1 analyzer append segments to
//! per-stream byte buffers in arrival order, so a reversed chunk order
//! corrupts the reconstructed stream. This reorderer holds each stream's
//! segments briefly and emits them in capture order, waiting only when a gap
//! suggests an earlier chunk may still arrive.
//!
//! The synchronous TLS probe is already serialized through a process-local
//! FIFO event queue and a single writer. Its producer sequence is
//! process-global, not stream-local, so interpreting gaps after per-stream
//! demultiplexing would create false gaps and can turn scheduling delay into a
//! late drop. Only `tls-sync:` segments bypass this cross-drain reorderer;
//! seccomp TLS capture retains its existing cross-completion ordering.
//!
//! # Why the added latency does not distort timing metrics
//!
//! The reorderer delays when a segment is *delivered* downstream, never the
//! timestamp attached to it. Segments carry `observed_at` derived from the
//! kernel capture timestamp (`bpf_ktime_get_ns`) at decode time; the reorderer
//! does not rewrite it. Downstream duration metrics are computed from capture
//! timestamps only:
//!
//! - `llm.response.start_time` / `end_time` come from the response segments'
//!   `observed_at` (`first`/`last`),
//! - `llm.call.end_time - llm.call.start_time` (the model's thinking time) is
//!   `response_last.observed_at - request_first.observed_at`.
//!
//! A 50 ms delivery delay therefore never enters a duration: the request and
//! response carry their true capture times regardless of when the reorderer
//! hands them over. Before the switch to capture-derived timestamps, delivery
//! delay *did* pollute the timeline — out-of-order consumption could make a
//! response's arrival precede its request's, yielding a negative duration.
//!
//! The one residual risk is the late-drop path (a chunk whose sequence falls
//! behind the already-emitted prefix is dropped because it can no longer be
//! inserted into the byte stream). If a response's final chunk is still
//! buffered when the trace is finalized, it is dropped and the response's
//! `end_time` may be missing or early — that call's duration is then
//! *incomplete*, never *wrong* (no fabricated timestamp). Drain cycles flush
//! expired chunks every poll and finalization has a settle delay, so this
//! window is tiny.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, SystemTime};

use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSourceBoundary, PayloadStreamKey};
use model_core::process::ProcessObservation;
use payload_event::RawPayloadSegment;
use payload_event::RawPayloadStreamClose;

/// How long to wait for an out-of-order predecessor before concluding the
/// intervening sequences were lost (capture gap, or a stream whose sequence
/// does not start at 1). Far larger than cross-CPU delivery jitter, far
/// smaller than typical inter-chunk intervals. This is a delivery delay only —
/// it does not affect any `observed_at`-derived duration metric (see module
/// docs).
const REORDER_WINDOW: Duration = Duration::from_millis(50);

/// The first sequence of eBPF socket payload streams.
const FIRST_SOCKET_SEQUENCE: u64 = 1;

/// Stream provenance assigned by the synchronous TLS event service.
const TLS_SYNC_STREAM_PREFIX: &str = "tls-sync:";

/// How long an emitted (idle) stream is retained before its ordering state is
/// dropped. Bounded by trace finalization, which forgets traces eagerly.
const STREAM_RETENTION: Duration = Duration::from_secs(60);

/// Orderable stand-in for `PayloadSourceBoundary` (the contract enum does not
/// implement `Ord`, which the reorder key needs).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ReorderBoundary {
    TlsUserSpace,
    Syscall,
    Stdio,
}

impl From<PayloadSourceBoundary> for ReorderBoundary {
    fn from(value: PayloadSourceBoundary) -> Self {
        match value {
            PayloadSourceBoundary::TlsUserSpace => Self::TlsUserSpace,
            PayloadSourceBoundary::Syscall => Self::Syscall,
            PayloadSourceBoundary::Stdio => Self::Stdio,
        }
    }
}

/// Orderable stand-in for `PayloadDirection`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ReorderDirection {
    Outbound,
    Inbound,
}

impl From<PayloadDirection> for ReorderDirection {
    fn from(value: PayloadDirection) -> Self {
        match value {
            PayloadDirection::Outbound => Self::Outbound,
            PayloadDirection::Inbound => Self::Inbound,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReorderStreamKey {
    trace_id: TraceId,
    process: ProcessObservation,
    source_boundary: ReorderBoundary,
    stream_key: PayloadStreamKey,
    direction: ReorderDirection,
}

struct ReorderStreamState {
    pending: BTreeMap<u64, VecDeque<RawPayloadSegment>>,
    last_emitted: Option<u64>,
    last_activity: SystemTime,
}

impl Default for ReorderStreamState {
    fn default() -> Self {
        Self {
            pending: BTreeMap::new(),
            last_emitted: None,
            last_activity: SystemTime::UNIX_EPOCH,
        }
    }
}

#[derive(Default)]
pub(crate) struct PayloadSegmentReorderer {
    streams: BTreeMap<ReorderStreamKey, ReorderStreamState>,
}

pub(crate) struct AdmittedPayloadSegment {
    pub(crate) segment: RawPayloadSegment,
    pub(crate) discontinuity_before: bool,
}

impl PayloadSegmentReorderer {
    /// Accept freshly captured segments and return kernel/seccomp segments
    /// whose capture order is now determined. Synchronous TLS segments retain
    /// FIFO arrival order. Other segments whose predecessors may still be in
    /// flight are held until `now` has passed `REORDER_WINDOW` beyond their
    /// observation time.
    pub(crate) fn admit(
        &mut self,
        now: SystemTime,
        segments: Vec<RawPayloadSegment>,
    ) -> Vec<AdmittedPayloadSegment> {
        if segments.is_empty() && self.streams.is_empty() {
            return Vec::new();
        }
        let mut ready = Vec::new();
        for segment in segments {
            if segment.source_boundary == PayloadSourceBoundary::TlsUserSpace
                && segment
                    .stream_key
                    .as_str()
                    .starts_with(TLS_SYNC_STREAM_PREFIX)
            {
                ready.push(AdmittedPayloadSegment {
                    segment,
                    discontinuity_before: false,
                });
                continue;
            }
            let key = reorder_stream_key(&segment);
            let state = self.streams.entry(key).or_default();
            state.last_activity = now;
            state
                .pending
                .entry(segment.sequence)
                .or_default()
                .push_back(segment);
        }
        for state in self.streams.values_mut() {
            drain_ready(state, now, &mut ready);
        }
        // Retain ordering state past a stream's last segment so a later
        // out-of-order chunk can still be placed correctly; drop it once it
        // has been idle long past the reorder window.
        self.streams.retain(|_, state| {
            !state.pending.is_empty()
                || now.duration_since(state.last_activity).unwrap_or_default() < STREAM_RETENTION
        });
        ready
    }

    pub(crate) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.trace_id != trace_id);
    }

    pub(crate) fn forget_stream(&mut self, close: &RawPayloadStreamClose) {
        self.streams.retain(|key, _| {
            key.trace_id != close.trace_id
                || key.process != close.process
                || key.source_boundary != close.source_boundary.into()
                || key.stream_key != close.stream_key
        });
    }

    #[cfg(test)]
    fn pending_segment_count(&self) -> usize {
        self.streams
            .values()
            .map(|state| state.pending.values().map(VecDeque::len).sum::<usize>())
            .sum()
    }
}

fn reorder_stream_key(segment: &RawPayloadSegment) -> ReorderStreamKey {
    ReorderStreamKey {
        trace_id: segment.trace_id,
        process: segment.process.clone(),
        source_boundary: segment.source_boundary.into(),
        stream_key: segment.stream_key.clone(),
        direction: segment.direction.into(),
    }
}

fn drain_ready(
    state: &mut ReorderStreamState,
    now: SystemTime,
    ready: &mut Vec<AdmittedPayloadSegment>,
) {
    let cutoff = now - REORDER_WINDOW;
    loop {
        let Some((&sequence, _)) = state.pending.iter().next() else {
            break;
        };
        // A chunk whose sequence is behind the last emitted one arrived too
        // late to be inserted into the byte stream — drop it.
        if state.last_emitted.is_some_and(|last| sequence < last) {
            state.pending.remove(&sequence);
            continue;
        }
        let continuous = match state.last_emitted {
            None => sequence == FIRST_SOCKET_SEQUENCE,
            Some(last) => sequence == last || sequence == last + 1,
        };
        let discontinuity_before = if !continuous {
            // There is a gap ahead of the emitted prefix. If the front chunk
            // is still fresh, an earlier sequence may be in flight — wait.
            let Some(front_observed_at) = state
                .pending
                .get(&sequence)
                .and_then(VecDeque::front)
                .map(|segment| segment.observed_at)
            else {
                break;
            };
            if front_observed_at > cutoff {
                break;
            }
            // The gap is old enough to conclude the intervening sequences
            // were lost; emit this chunk as the new anchor.
            true
        } else {
            false
        };
        let segment = state
            .pending
            .get_mut(&sequence)
            .expect("pending sequence just looked up")
            .pop_front()
            .expect("pending queue just checked non-empty");
        state.last_emitted = Some(sequence.max(state.last_emitted.unwrap_or(0)));
        state.last_activity = now;
        let discontinuity_before = discontinuity_before
            && segment.source_boundary == PayloadSourceBoundary::Syscall
            && segment.library == "socket-syscall";
        ready.push(AdmittedPayloadSegment {
            segment,
            discontinuity_before,
        });
        if state.pending.get(&sequence).is_none_or(VecDeque::is_empty) {
            state.pending.remove(&sequence);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use model_core::payload::{
        PayloadContentState, PayloadOperationCompletionState, PayloadSourceBoundary,
        PayloadStreamKey, PayloadTruncationState,
    };
    use model_core::process::ProcessObservation;

    use super::*;

    fn segment(
        sequence: u64,
        observed_at: SystemTime,
        stream_key: &str,
        direction: PayloadDirection,
    ) -> RawPayloadSegment {
        RawPayloadSegment {
            trace_id: TraceId::new(1),
            observed_at,
            process: ProcessObservation::host(model_core::process::HostProcessCoordinates::new(
                42, 0,
            )),
            source_boundary: PayloadSourceBoundary::Syscall,
            content_state: PayloadContentState::Plaintext,
            direction,
            stream_key: PayloadStreamKey::new(stream_key.to_string()),
            sequence,
            original_size: 1,
            captured_size: 1,
            operation_id: sequence,
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
}
