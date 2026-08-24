use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxAgentSnapshot {
    pub sb_id: u32,
    pub collected_io_observations: u64,
    pub collected_resource_observations: u64,
    pub source_failures: u64,
    pub dropped_observations: u64,
    pub sent_batches: u64,
    pub reconnects: u64,
    pub reconnect_failures: u64,
}

pub(crate) struct DaemonMetrics {
    io_observations: AtomicU64,
    resource_observations: AtomicU64,
    source_failures: AtomicU64,
    dropped_observations: AtomicU64,
    sent_batches: AtomicU64,
    reconnects: AtomicU64,
    reconnect_failures: AtomicU64,
    sb_id: AtomicU32,
    enabled: bool,
}

impl DaemonMetrics {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            io_observations: AtomicU64::new(0),
            resource_observations: AtomicU64::new(0),
            source_failures: AtomicU64::new(0),
            dropped_observations: AtomicU64::new(0),
            sent_batches: AtomicU64::new(0),
            reconnects: AtomicU64::new(0),
            reconnect_failures: AtomicU64::new(0),
            sb_id: AtomicU32::new(0),
            enabled,
        }
    }

    pub(crate) fn record_observations(&self, io_source: bool, accepted: u64, dropped: u64) {
        if !self.enabled {
            return;
        }
        let counter = if io_source {
            &self.io_observations
        } else {
            &self.resource_observations
        };
        counter.fetch_add(accepted, Ordering::Relaxed);
        self.dropped_observations
            .fetch_add(dropped, Ordering::Relaxed);
    }

    pub(crate) fn record_source_failure(&self) {
        if self.enabled {
            self.source_failures.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn set_sb_id(&self, sb_id: u32) {
        self.sb_id.store(sb_id, Ordering::Release);
    }

    pub(crate) fn record_sent_batch(&self) {
        if self.enabled {
            self.sent_batches.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_reconnect(&self) {
        if self.enabled {
            self.reconnects.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_reconnect_failure(&self) {
        if self.enabled {
            let _ = self.reconnect_failures.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |value| value.checked_add(1),
            );
        }
    }

    pub(crate) fn snapshot(&self) -> SandboxAgentSnapshot {
        SandboxAgentSnapshot {
            sb_id: self.sb_id.load(Ordering::Acquire),
            collected_io_observations: self.io_observations.load(Ordering::Relaxed),
            collected_resource_observations: self.resource_observations.load(Ordering::Relaxed),
            source_failures: self.source_failures.load(Ordering::Relaxed),
            dropped_observations: self.dropped_observations.load(Ordering::Relaxed),
            sent_batches: self.sent_batches.load(Ordering::Relaxed),
            reconnects: self.reconnects.load(Ordering::Relaxed),
            reconnect_failures: self.reconnect_failures.load(Ordering::Relaxed),
        }
    }
}
