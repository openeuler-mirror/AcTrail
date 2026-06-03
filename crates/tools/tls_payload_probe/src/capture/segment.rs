//! Reassembles segmented BPF payload records into logical TLS operations.

use std::collections::HashMap;

use crate::capture::config::ABI_MAX_CAPTURE_BYTES;
use crate::capture::event::{CaptureDirection, CaptureEvent, CaptureFlags};
use crate::capture::ring_stats::payload_class;
use crate::{ToolError, ToolResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PayloadSegment {
    pub(super) pid: u32,
    pub(super) tid: u32,
    pub(super) provider: String,
    pub(super) symbol: String,
    pub(super) direction: CaptureDirection,
    pub(super) requested_size: u64,
    pub(super) observed_ktime_ns: u64,
    pub(super) stream_key: u64,
    pub(super) segment_offset: u64,
    pub(super) operation_size: u64,
    pub(super) flags: CaptureFlags,
    pub(super) captured: Vec<u8>,
}

#[derive(Default)]
pub(super) struct PayloadSegmentAssembler {
    pending: HashMap<SegmentKey, PendingOperation>,
}

impl PayloadSegmentAssembler {
    pub(super) fn push(&mut self, segment: PayloadSegment) -> ToolResult<Vec<CaptureEvent>> {
        let placement = SegmentPlacement::new(&segment)?;
        if placement.start == 0 && placement.end == placement.operation_size {
            let ring_size = segment.captured.len();
            let reserved_size = reserved_payload_size(&segment);
            return Ok(vec![
                segment.into_event(vec![ring_size], vec![reserved_size]),
            ]);
        }

        let key = SegmentKey::from(&segment);
        let pending = self
            .pending
            .entry(key.clone())
            .or_insert_with(|| PendingOperation::new(placement.operation_size));
        pending.push(&segment, placement)?;
        if !pending.is_complete() {
            return Ok(Vec::new());
        }

        let pending = self
            .pending
            .remove(&key)
            .expect("pending operation was checked above");
        Ok(vec![pending.into_event(key)])
    }

    pub(super) fn finish(&mut self) -> ToolResult<Vec<CaptureEvent>> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let (key, pending) = self
            .pending
            .iter()
            .next()
            .expect("pending map is known to be non-empty");
        Err(ToolError::new(format!(
            "incomplete segmented TLS payload operation pid={} tid={} stream=0x{:x} symbol={} received={} expected={}",
            key.pid,
            key.tid,
            key.stream_key,
            key.symbol,
            pending.received_bytes,
            pending.operation_size
        )))
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SegmentKey {
    pid: u32,
    tid: u32,
    provider: String,
    symbol: String,
    direction: CaptureDirection,
    requested_size: u64,
    observed_ktime_ns: u64,
    stream_key: u64,
    operation_size: usize,
}

impl From<&PayloadSegment> for SegmentKey {
    fn from(segment: &PayloadSegment) -> Self {
        Self {
            pid: segment.pid,
            tid: segment.tid,
            provider: segment.provider.clone(),
            symbol: segment.symbol.clone(),
            direction: segment.direction,
            requested_size: segment.requested_size,
            observed_ktime_ns: segment.observed_ktime_ns,
            stream_key: segment.stream_key,
            operation_size: segment.operation_size as usize,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SegmentPlacement {
    start: usize,
    end: usize,
    operation_size: usize,
}

impl SegmentPlacement {
    fn new(segment: &PayloadSegment) -> ToolResult<Self> {
        let operation_size = usize::try_from(segment.operation_size)
            .map_err(|error| ToolError::new(format!("payload operation size overflow: {error}")))?;
        if operation_size == 0 {
            return Err(ToolError::new("payload segment operation_size is zero"));
        }
        let start = usize::try_from(segment.segment_offset)
            .map_err(|error| ToolError::new(format!("payload segment offset overflow: {error}")))?;
        let end = start
            .checked_add(segment.captured.len())
            .ok_or_else(|| ToolError::new("payload segment end overflow"))?;
        if start >= operation_size || end > operation_size {
            return Err(ToolError::new(format!(
                "payload segment range {}..{} exceeds operation size {}",
                start, end, operation_size
            )));
        }
        Ok(Self {
            start,
            end,
            operation_size,
        })
    }
}

struct PendingOperation {
    buffer: Vec<u8>,
    ranges: Vec<(usize, usize)>,
    flags: CaptureFlags,
    received_bytes: usize,
    operation_size: usize,
    ring_captured_sizes: Vec<usize>,
    ring_reserved_sizes: Vec<usize>,
}

impl PendingOperation {
    fn new(operation_size: usize) -> Self {
        Self {
            buffer: vec![0; operation_size],
            ranges: Vec::new(),
            flags: CaptureFlags {
                truncated: false,
                rustls_chunk: false,
            },
            received_bytes: 0,
            operation_size,
            ring_captured_sizes: Vec::new(),
            ring_reserved_sizes: Vec::new(),
        }
    }

    fn push(&mut self, segment: &PayloadSegment, placement: SegmentPlacement) -> ToolResult<()> {
        for (start, end) in &self.ranges {
            if placement.start < *end && placement.end > *start {
                return Err(ToolError::new(format!(
                    "overlapping TLS payload segment range {}..{} conflicts with {}..{}",
                    placement.start, placement.end, start, end
                )));
            }
        }
        self.buffer[placement.start..placement.end].copy_from_slice(&segment.captured);
        self.ranges.push((placement.start, placement.end));
        self.flags.truncated |= segment.flags.truncated;
        self.flags.rustls_chunk |= segment.flags.rustls_chunk;
        self.received_bytes += segment.captured.len();
        self.ring_captured_sizes.push(segment.captured.len());
        self.ring_reserved_sizes
            .push(reserved_payload_size(segment));
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.received_bytes == self.operation_size
    }

    fn into_event(self, key: SegmentKey) -> CaptureEvent {
        CaptureEvent {
            pid: key.pid,
            tid: key.tid,
            provider: key.provider,
            symbol: key.symbol,
            direction: key.direction,
            requested_size: key.requested_size,
            observed_ktime_ns: key.observed_ktime_ns,
            stream_key: key.stream_key,
            flags: self.flags,
            captured: self.buffer,
            ring_captured_sizes: self.ring_captured_sizes,
            ring_reserved_sizes: self.ring_reserved_sizes,
        }
    }
}

impl PayloadSegment {
    fn into_event(
        self,
        ring_captured_sizes: Vec<usize>,
        ring_reserved_sizes: Vec<usize>,
    ) -> CaptureEvent {
        CaptureEvent {
            pid: self.pid,
            tid: self.tid,
            provider: self.provider,
            symbol: self.symbol,
            direction: self.direction,
            requested_size: self.requested_size,
            observed_ktime_ns: self.observed_ktime_ns,
            stream_key: self.stream_key,
            flags: self.flags,
            captured: self.captured,
            ring_captured_sizes,
            ring_reserved_sizes,
        }
    }
}

fn reserved_payload_size(segment: &PayloadSegment) -> usize {
    let is_segmented =
        segment.operation_size > segment.captured.len() as u64 || segment.segment_offset > 0;
    if is_segmented {
        ABI_MAX_CAPTURE_BYTES
    } else {
        payload_class(segment.captured.len())
    }
}
