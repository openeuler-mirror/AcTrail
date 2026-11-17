//! Bounded delivery, retries, and buffered flushes.

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use export_core::{BestEffortShutdown, BestEffortSink};

use crate::batch::{EncodedRecord, PendingBatch};
use crate::config::{
    Endpoint, OtelCompression, OtelEncoding, OtelHttpExporterConfig, OtelHttpTlsConfig,
};
use crate::live_http::OTEL_HTTP_EXPORTER_NAME;
use crate::metrics::OtelHttpOperationalMetrics;
use crate::transport::{
    HttpConnection, PostError, PostRequest, post_otlp_reusing, render_extra_headers,
};

/// Batches rendered OTLP documents and POSTs them to the collector.
///
/// Failure policy (deliberate, documented): a batch gets bounded retries with
/// backoff; when they are exhausted the batch is dropped, logged loudly, and
/// the route STAYS ALIVE — a collector outage must degrade to data loss, not
/// kill live export for the rest of the daemon's life. `finish()` (called on
/// route shutdown after the queue drains) flushes the tail under a deadline so
/// shutdown callers are never blocked indefinitely.
pub(super) struct HttpBatchSink {
    endpoint: Endpoint,
    batch_max_spans: usize,
    batch_timeout: Duration,
    /// When the oldest buffered span was appended; `None` when the buffer is
    /// empty. The delivery worker uses this to schedule an idle flush.
    pending_since: Option<Instant>,
    pending: PendingBatch,
    encoding: OtelEncoding,
    compression: OtelCompression,
    connect_timeout: Duration,
    request_timeout: Duration,
    retry_max_attempts: u32,
    retry_backoff: Duration,
    shutdown_flush_deadline: Duration,
    shutdown: BestEffortShutdown,
    tls: OtelHttpTlsConfig,
    /// Configured extra headers, pre-rendered once as `name: value\r\n` lines
    /// and spliced verbatim into every request head. Empty when unconfigured.
    extra_headers: String,
    connection: Option<HttpConnection>,
    dropped_batches: u64,
    metrics: Arc<OtelHttpOperationalMetrics>,
}

impl HttpBatchSink {
    pub(super) fn new_with_metrics(
        endpoint: Endpoint,
        config: &OtelHttpExporterConfig,
        metrics: Arc<OtelHttpOperationalMetrics>,
    ) -> Self {
        Self {
            endpoint,
            batch_max_spans: config.batch_max_spans as usize,
            batch_timeout: Duration::from_millis(u64::from(config.batch_timeout_ms)),
            pending_since: None,
            pending: PendingBatch::new(config.encoding),
            encoding: config.encoding,
            compression: config.compression,
            connect_timeout: config.connect_timeout(),
            request_timeout: config.request_timeout(),
            retry_max_attempts: config.retry_max_attempts,
            retry_backoff: config.retry_backoff(),
            shutdown_flush_deadline: config.shutdown_flush_deadline(),
            shutdown: BestEffortShutdown::default(),
            tls: config.tls.clone(),
            extra_headers: render_extra_headers(&config.headers),
            connection: None,
            dropped_batches: 0,
            metrics,
        }
    }

    /// Deliver everything buffered, splitting any batch the collector rejects
    /// as too large rather than dropping spans that simply did not fit.
    fn flush_batch(&mut self, deadline: Option<Instant>) -> Result<u64, String> {
        if self.pending.is_empty() {
            return Ok(u64::default());
        }
        let mut remaining = vec![std::mem::replace(
            &mut self.pending,
            PendingBatch::new(self.encoding),
        )];
        let mut durable = u64::default();
        while let Some(mut batch) = remaining.pop() {
            match self.deliver_batch(&batch, deadline)? {
                BatchOutcome::Delivered(spans) => durable = durable.saturating_add(spans),
                BatchOutcome::TooLarge => {
                    if let Some(tail) = batch.split_off_half() {
                        // The collector's body limit is below our batch size:
                        // shrink later batches too, or every flush pays this.
                        self.batch_max_spans = (self.batch_max_spans / 2).max(1);
                        // Pushed tail-first so the halves keep their order.
                        remaining.push(tail);
                        remaining.push(batch);
                        continue;
                    }
                    self.drop_batch(
                        &batch,
                        "collector rejected an indivisible batch as too large",
                    );
                }
                BatchOutcome::Dropped(detail) => self.drop_batch(&batch, &detail),
            }
        }
        self.pending_since = None;
        self.metrics.set_pending(0, None);
        Ok(durable)
    }

    /// Drop one batch, keeping the route alive. Best-effort delivery's only
    /// failure mode, and it is always loud.
    fn drop_batch(&mut self, batch: &PendingBatch, detail: &str) {
        let batch_spans = batch.len();
        self.dropped_batches = self.dropped_batches.saturating_add(1);
        self.metrics.record_dropped_batch(batch_spans);
        eprintln!(
            "{OTEL_HTTP_EXPORTER_NAME}: dropped batch of {batch_spans} spans after {} attempts \
             (total dropped batches {}): {detail}",
            self.retry_max_attempts, self.dropped_batches,
        );
    }

    /// Run one batch's attempt sequence. `Err` is an encoding failure, which no
    /// retry or split can fix.
    fn deliver_batch(
        &mut self,
        batch: &PendingBatch,
        deadline: Option<Instant>,
    ) -> Result<BatchOutcome, String> {
        let batch_spans = batch.len();
        let body = batch
            .body()
            .and_then(|body| encode_request_body(&body, self.compression))
            .inspect_err(|error| {
                self.metrics.record_error(error.clone());
            })?;
        let content_type = self.encoding.content_type();
        let content_encoding = self.compression.content_encoding();
        let mut last_error = String::new();
        for attempt in 0..self.retry_max_attempts {
            let effective_deadline = earliest_deadline(deadline, self.shutdown.deadline());
            if self.shutdown.expired()
                || effective_deadline.is_some_and(|deadline| Instant::now() >= deadline)
            {
                break;
            }
            match post_otlp_reusing(
                &mut self.connection,
                PostRequest {
                    endpoint: &self.endpoint,
                    extra_headers: &self.extra_headers,
                    body: &body,
                    content_type,
                    content_encoding,
                    connect_timeout: self.connect_timeout,
                    request_timeout: self.request_timeout,
                    tls: &self.tls,
                },
            ) {
                Ok(success) => {
                    self.metrics.record_success(success.partial_rejected);
                    let durable = u64::try_from(batch_spans)
                        .unwrap_or(u64::MAX)
                        .saturating_sub(success.partial_rejected);
                    return Ok(BatchOutcome::Delivered(durable));
                }
                // The body was too big, not wrong: hand it back to be split.
                Err(PostError::TooLarge { detail }) => {
                    self.metrics.record_error(detail);
                    return Ok(BatchOutcome::TooLarge);
                }
                // OTLP: 400 and other non-retryable statuses will never succeed on
                // replay — drop immediately instead of burning the retry budget.
                Err(PostError::Permanent { detail }) => {
                    last_error = detail;
                    self.metrics.record_error(last_error.clone());
                    break;
                }
                // Only transient statuses and transport errors are retried;
                // honor a server `Retry-After`, otherwise exponential backoff+jitter.
                Err(PostError::Retryable {
                    detail,
                    retry_after,
                }) => {
                    last_error = detail;
                    self.metrics.record_error(last_error.clone());
                    let effective_deadline = earliest_deadline(deadline, self.shutdown.deadline());
                    if attempt + 1 < self.retry_max_attempts
                        && !self.shutdown.expired()
                        && !effective_deadline.is_some_and(|deadline| Instant::now() >= deadline)
                    {
                        self.metrics.record_retry();
                        let wait = retry_after
                            .unwrap_or_else(|| backoff_with_jitter(self.retry_backoff, attempt));
                        std::thread::sleep(bounded_backoff(wait, effective_deadline));
                    }
                }
            }
        }
        if last_error.is_empty() {
            last_error = "shutdown flush deadline exceeded".to_string();
            self.metrics.record_error(last_error.clone());
        }
        Ok(BatchOutcome::Dropped(last_error))
    }

    /// A partial batch has waited at least `batch_timeout` since its first span.
    fn batch_timed_out(&self) -> bool {
        self.pending_since
            .is_some_and(|since| since.elapsed() >= self.batch_timeout)
    }
}

impl BestEffortSink<EncodedRecord> for HttpBatchSink {
    fn bind_shutdown(&mut self, shutdown: BestEffortShutdown) {
        self.shutdown = shutdown;
    }

    fn deliver(&mut self, record: EncodedRecord) -> Result<u64, String> {
        self.metrics.queue_leave();
        let was_empty = self.pending.is_empty();
        self.pending.append(record);
        if was_empty && !self.pending.is_empty() {
            // Start the batch clock at the first buffered span.
            self.pending_since = Some(Instant::now());
        }
        self.metrics
            .set_pending(self.pending.len(), self.pending_since);
        if self.pending.len() >= self.batch_max_spans || self.batch_timed_out() {
            return self.flush_batch(None);
        }
        Ok(u64::default())
    }

    fn idle_timeout(&self) -> Option<Duration> {
        self.pending_since
            .map(|since| self.batch_timeout.saturating_sub(since.elapsed()))
    }

    fn on_idle(&mut self) -> Result<u64, String> {
        self.flush_batch(None)
    }

    fn finish(&mut self) -> Result<u64, String> {
        let local_deadline = Instant::now() + self.shutdown_flush_deadline;
        let deadline = earliest_deadline(Some(local_deadline), self.shutdown.deadline())
            .unwrap_or(local_deadline);
        self.flush_batch(Some(deadline))
    }
}

fn earliest_deadline(left: Option<Instant>, right: Option<Instant>) -> Option<Instant> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    }
}

fn bounded_backoff(backoff: Duration, deadline: Option<Instant>) -> Duration {
    match deadline {
        Some(deadline) => backoff.min(deadline.saturating_duration_since(Instant::now())),
        None => backoff,
    }
}

/// What one batch's delivery attempt sequence concluded.
enum BatchOutcome {
    /// Accepted by the collector; carries the span count the collector kept.
    Delivered(u64),
    /// Rejected as too large — the caller splits and redelivers.
    TooLarge,
    /// Retries exhausted or permanently rejected; carries the last error.
    Dropped(String),
}

/// Exponential backoff with equal jitter: wait lands in `[ceil/2, ceil]` where
/// `ceil = min(base * 2^attempt, 30s)`. Jitter avoids retry stampedes; the
/// randomness source is a cheap non-cryptographic clock mix (no `rand` dep).
fn backoff_with_jitter(base: Duration, attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
    let ceiling = base.saturating_mul(factor).min(Duration::from_secs(30));
    let half = ceiling / 2;
    half + half.mul_f64(pseudo_random_fraction())
}

/// A pseudo-random fraction in `[0.0, 1.0)` from the sub-second clock. Not
/// cryptographic — only used to spread retry timings.
fn pseudo_random_fraction() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    let mixed = nanos.wrapping_mul(2_654_435_761);
    f64::from(mixed) / f64::from(u32::MAX)
}

fn encode_request_body(body: &[u8], compression: OtelCompression) -> Result<Vec<u8>, String> {
    match compression {
        OtelCompression::None => Ok(body.to_vec()),
        OtelCompression::Gzip => {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder
                .write_all(body)
                .map_err(|error| format!("gzip OTLP request: {error}"))?;
            encoder
                .finish()
                .map_err(|error| format!("finish gzip OTLP request: {error}"))
        }
    }
}
