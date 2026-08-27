use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use sandbox_evidence_store::{
    SandboxEvidenceHealth, SandboxEvidenceStatus, SandboxEvidenceStatusPort,
};

const HEALTH_READY: u8 = 1;
const HEALTH_DEGRADED: u8 = 2;
const HEALTH_STOPPING: u8 = 3;
const HEALTH_STOPPED: u8 = 4;
const HEALTH_FAILED: u8 = 5;

pub(super) struct StoreStatus {
    schema_version: u32,
    pub(super) ingest_epoch: AtomicU64,
    queue_capacity: u32,
    pub(super) stopping: AtomicBool,
    health: AtomicU8,
    pub(super) queue_depth: AtomicU64,
    pub(super) accepted_batches: AtomicU64,
    pub(super) accepted_observations: AtomicU64,
    pub(super) rejected_batches: AtomicU64,
    pub(super) committed_batches: AtomicU64,
    pub(super) committed_observations: AtomicU64,
    pub(super) duplicate_observations: AtomicU64,
    pub(super) failed_batches: AtomicU64,
    pub(super) retained_observations: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl StoreStatus {
    pub(super) fn new(schema_version: u32, queue_capacity: u32) -> Self {
        Self {
            schema_version,
            ingest_epoch: AtomicU64::new(0),
            queue_capacity,
            stopping: AtomicBool::new(false),
            health: AtomicU8::new(HEALTH_FAILED),
            queue_depth: AtomicU64::new(0),
            accepted_batches: AtomicU64::new(0),
            accepted_observations: AtomicU64::new(0),
            rejected_batches: AtomicU64::new(0),
            committed_batches: AtomicU64::new(0),
            committed_observations: AtomicU64::new(0),
            duplicate_observations: AtomicU64::new(0),
            failed_batches: AtomicU64::new(0),
            retained_observations: AtomicU64::new(0),
            last_error: Mutex::new(None),
        }
    }

    pub(super) fn mark_ready(&self, ingest_epoch: u64) {
        self.ingest_epoch.store(ingest_epoch, Ordering::Release);
        self.health.store(HEALTH_READY, Ordering::Release);
    }

    pub(super) fn mark_stopping(&self) {
        self.stopping.store(true, Ordering::Release);
        self.health.store(HEALTH_STOPPING, Ordering::Release);
    }

    pub(super) fn mark_stopped(&self) {
        self.health.store(HEALTH_STOPPED, Ordering::Release);
    }

    pub(super) fn record_success(&self) {
        if !self.stopping.load(Ordering::Acquire) {
            self.health.store(HEALTH_READY, Ordering::Release);
        }
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = None;
        }
    }

    pub(super) fn record_failure(&self, message: impl Into<String>, fatal: bool) {
        self.health.store(
            if fatal {
                HEALTH_FAILED
            } else {
                HEALTH_DEGRADED
            },
            Ordering::Release,
        );
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = Some(message.into());
        }
    }

    fn health(&self) -> SandboxEvidenceHealth {
        match self.health.load(Ordering::Acquire) {
            HEALTH_READY => SandboxEvidenceHealth::Ready,
            HEALTH_DEGRADED => SandboxEvidenceHealth::Degraded,
            HEALTH_STOPPING => SandboxEvidenceHealth::Stopping,
            HEALTH_STOPPED => SandboxEvidenceHealth::Stopped,
            _ => SandboxEvidenceHealth::Failed,
        }
    }
}

impl SandboxEvidenceStatusPort for StoreStatus {
    fn status(&self) -> SandboxEvidenceStatus {
        SandboxEvidenceStatus {
            schema_version: self.schema_version,
            ingest_epoch: self.ingest_epoch.load(Ordering::Acquire),
            health: self.health(),
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            queue_capacity: self.queue_capacity,
            accepted_batches: self.accepted_batches.load(Ordering::Relaxed),
            accepted_observations: self.accepted_observations.load(Ordering::Relaxed),
            rejected_batches: self.rejected_batches.load(Ordering::Relaxed),
            committed_batches: self.committed_batches.load(Ordering::Relaxed),
            committed_observations: self.committed_observations.load(Ordering::Relaxed),
            duplicate_observations: self.duplicate_observations.load(Ordering::Relaxed),
            failed_batches: self.failed_batches.load(Ordering::Relaxed),
            retained_observations: self.retained_observations.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|error| error.clone()),
        }
    }
}
