//! Operational counters shared by the consumer and delivery worker.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use plugin_system::{PluginOperationalMetrics, PluginOperationalMetricsSource};

pub(super) struct OtelHttpOperationalMetrics {
    queue_depth: AtomicU64,
    queue_capacity: u32,
    pending_spans: AtomicU64,
    dropped_batches: AtomicU64,
    worker_dropped_spans: AtomicU64,
    inline_dropped_spans: AtomicU64,
    finish_reported_drops: AtomicU64,
    retry_attempts: AtomicU64,
    successful_batches: AtomicU64,
    partial_rejected_spans: AtomicU64,
    last_success_unix_ms: AtomicU64,
    pending_since: Mutex<Option<Instant>>,
    last_error: Mutex<Option<String>>,
}

impl OtelHttpOperationalMetrics {
    pub(super) fn new(queue_capacity: u32) -> Self {
        Self {
            queue_depth: AtomicU64::new(0),
            queue_capacity,
            pending_spans: AtomicU64::new(0),
            dropped_batches: AtomicU64::new(0),
            worker_dropped_spans: AtomicU64::new(0),
            inline_dropped_spans: AtomicU64::new(0),
            finish_reported_drops: AtomicU64::new(0),
            retry_attempts: AtomicU64::new(0),
            successful_batches: AtomicU64::new(0),
            partial_rejected_spans: AtomicU64::new(0),
            last_success_unix_ms: AtomicU64::new(0),
            pending_since: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    pub(super) fn queue_enter(&self) {
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn queue_leave(&self) {
        let _ = self
            .queue_depth
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                Some(depth.saturating_sub(1))
            });
    }

    pub(super) fn set_pending(&self, spans: usize, since: Option<Instant>) {
        self.pending_spans
            .store(u64::try_from(spans).unwrap_or(u64::MAX), Ordering::Relaxed);
        if let Ok(mut slot) = self.pending_since.lock() {
            *slot = since;
        }
    }

    pub(super) fn record_retry(&self) {
        self.retry_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_error(&self, error: impl Into<String>) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(error.into());
        }
    }

    pub(super) fn record_success(&self, partial_rejected: u64) {
        self.successful_batches.fetch_add(1, Ordering::Relaxed);
        if partial_rejected > 0 {
            self.partial_rejected_spans
                .fetch_add(partial_rejected, Ordering::Relaxed);
            self.worker_dropped_spans
                .fetch_add(partial_rejected, Ordering::Relaxed);
        }
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default();
        self.last_success_unix_ms.store(unix_ms, Ordering::Relaxed);
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = None;
        }
    }

    // The host counts these consume-report rejections. They contribute to the
    // named total, but not operational dropped_records, which the host adds.
    pub(super) fn record_inline_drop(&self, count: u64) {
        self.inline_dropped_spans
            .fetch_add(count, Ordering::Relaxed);
    }

    pub(super) fn report_finish_drops(&self, count: u64) {
        // The host now counts unacknowledged records, including worker losses
        // reported after a worker is detached at the shutdown deadline.
        self.finish_reported_drops
            .fetch_max(count, Ordering::Relaxed);
    }

    pub(super) fn record_dropped_batch(&self, spans: usize) {
        self.dropped_batches.fetch_add(1, Ordering::Relaxed);
        self.worker_dropped_spans
            .fetch_add(u64::try_from(spans).unwrap_or(u64::MAX), Ordering::Relaxed);
    }
}

impl PluginOperationalMetricsSource for OtelHttpOperationalMetrics {
    fn snapshot(&self) -> PluginOperationalMetrics {
        let oldest_pending_age_ms = self
            .pending_since
            .lock()
            .ok()
            .and_then(|since| *since)
            .map(|since| u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default();
        let worker_drops = self.worker_dropped_spans.load(Ordering::Relaxed);
        let inline_drops = self.inline_dropped_spans.load(Ordering::Relaxed);
        let finish_drops = self.finish_reported_drops.load(Ordering::Relaxed);
        let delivery_drops = worker_drops.max(finish_drops);
        let values = BTreeMap::from([
            (
                "otel_http.dropped_spans".into(),
                delivery_drops.saturating_add(inline_drops),
            ),
            ("otel_http.inline_dropped_spans".into(), inline_drops),
            ("otel_http.worker_dropped_spans".into(), delivery_drops),
            ("otel_http.finish_reported_drops".into(), finish_drops),
            (
                "otel_http.dropped_batches".to_string(),
                self.dropped_batches.load(Ordering::Relaxed),
            ),
            (
                "otel_http.last_success_unix_ms".to_string(),
                self.last_success_unix_ms.load(Ordering::Relaxed),
            ),
            (
                "otel_http.oldest_pending_age_ms".to_string(),
                oldest_pending_age_ms,
            ),
            (
                "otel_http.partial_rejected_spans".to_string(),
                self.partial_rejected_spans.load(Ordering::Relaxed),
            ),
            (
                "otel_http.pending_spans".to_string(),
                self.pending_spans.load(Ordering::Relaxed),
            ),
            (
                "otel_http.retry_attempts".to_string(),
                self.retry_attempts.load(Ordering::Relaxed),
            ),
            (
                "otel_http.successful_batches".to_string(),
                self.successful_batches.load(Ordering::Relaxed),
            ),
        ]);
        PluginOperationalMetrics {
            queue_depth: Some(self.queue_depth.load(Ordering::Relaxed)),
            queue_capacity: Some(self.queue_capacity),
            dropped_records: worker_drops.saturating_sub(finish_drops),
            last_error: self.last_error.lock().ok().and_then(|error| error.clone()),
            values,
        }
    }
}
