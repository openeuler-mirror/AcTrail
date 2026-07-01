//! Ring-buffer payload accounting.

use crate::capture::CaptureEvent;

const EVENT_HEADER_BYTES: usize = 72;
const MAX_PAYLOAD_CLASS: usize = 65535;
const PAYLOAD_CLASSES: &[usize] = &[512, 2048, 4096, 8192, MAX_PAYLOAD_CLASS];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RingStatsCollector {
    emitted: RingEmittedStats,
}

impl RingStatsCollector {
    pub(crate) fn observe(&mut self, event: &CaptureEvent) {
        for (captured_size, reserved_size) in event
            .ring_captured_sizes
            .iter()
            .zip(event.ring_reserved_sizes.iter())
        {
            self.emitted.observe(*captured_size, *reserved_size);
        }
    }

    pub(crate) fn finish(self, lost: RingLostStats) -> RingStatsSnapshot {
        RingStatsSnapshot {
            emitted: self.emitted,
            lost,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RingStatsSnapshot {
    pub(crate) emitted: RingEmittedStats,
    pub(crate) lost: RingLostStats,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RingEmittedStats {
    pub(crate) events: u64,
    pub(crate) actual_bytes: u64,
    pub(crate) reserved_bytes: u64,
    buckets: Vec<RingBucketStats>,
}

impl RingEmittedStats {
    pub(crate) fn slack_bytes(&self) -> u64 {
        self.reserved_bytes.saturating_sub(self.actual_bytes)
    }

    pub(crate) fn buckets(&self) -> &[RingBucketStats] {
        &self.buckets
    }

    fn observe(&mut self, captured_bytes: usize, reserved_bytes: usize) {
        self.events += 1;
        self.actual_bytes += record_bytes(captured_bytes) as u64;
        self.reserved_bytes += record_bytes(reserved_bytes) as u64;
        let index = self
            .buckets
            .iter()
            .position(|bucket| bucket.class_size == reserved_bytes)
            .unwrap_or_else(|| {
                self.buckets.push(RingBucketStats {
                    class_size: reserved_bytes,
                    events: 0,
                    actual_bytes: 0,
                    reserved_bytes: 0,
                });
                self.buckets.len() - 1
            });
        let bucket = &mut self.buckets[index];
        bucket.events += 1;
        bucket.actual_bytes += record_bytes(captured_bytes) as u64;
        bucket.reserved_bytes += record_bytes(reserved_bytes) as u64;
    }
}

impl Default for RingEmittedStats {
    fn default() -> Self {
        Self {
            events: 0,
            actual_bytes: 0,
            reserved_bytes: 0,
            buckets: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RingBucketStats {
    pub(crate) class_size: usize,
    pub(crate) events: u64,
    pub(crate) actual_bytes: u64,
    pub(crate) reserved_bytes: u64,
}

impl RingBucketStats {
    pub(crate) fn slack_bytes(&self) -> u64 {
        self.reserved_bytes.saturating_sub(self.actual_bytes)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RingLostStats {
    pub(crate) reserve_fail_events: u64,
    pub(crate) reserve_fail_actual_bytes: u64,
    pub(crate) reserve_fail_reserved_bytes: u64,
    pub(crate) read_user_fail_events: u64,
    pub(crate) read_user_fail_actual_bytes: u64,
    pub(crate) read_user_fail_reserved_bytes: u64,
    pub(crate) output_fail_events: u64,
    pub(crate) output_fail_actual_bytes: u64,
    pub(crate) output_fail_reserved_bytes: u64,
    pub(crate) perf_lost_events: u64,
}

pub(crate) fn payload_class(captured_bytes: usize) -> usize {
    PAYLOAD_CLASSES
        .iter()
        .copied()
        .find(|class_size| captured_bytes <= *class_size)
        .unwrap_or(MAX_PAYLOAD_CLASS)
}

fn record_bytes(payload_bytes: usize) -> usize {
    EVENT_HEADER_BYTES + payload_bytes
}
