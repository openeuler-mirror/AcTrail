//! Realtime OTLP/HTTP exporter: batches rendered spans and POSTs them to a
//! Collector from host, container, or virtual-machine deployments. The route
//! is bounded in memory and in how long its caller waits during shutdown.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use export_core::{
    BestEffortSemanticActionRoute, BestEffortSemanticActionRouteConfig, BestEffortShutdown,
    BestEffortSink, ExportError, SemanticActionExportAdapter, SemanticActionExportRecord,
    SemanticActionExportRoute, SemanticActionKindSelection,
};
use plugin_system::{
    DEFAULT_OBSERVATION_EVENT_FAMILIES, ObservationBatch, ObservationConsumeReport,
    ObservationConsumer, ObservationEventFamily, PluginDroppedRecord, PluginOperationalMetrics,
    PluginOperationalMetricsSource, PluginRuntimeError, PluginRuntimeKind,
};

use crate::config::{
    Endpoint, OtelAttributeMode, OtelCompression, OtelEncoding, OtelHttpExporterConfig,
    OtelHttpTlsConfig,
};

const OTEL_HTTP_EXPORTER_NAME: &str = "otel_live_http";
const OTEL_HTTP_PLUGIN_ID: &str = "otel-http";
pub const OTEL_HTTP_BUILTIN_PLUGIN_INSTANCE_ID: &str = "builtin.otel-http";
const SENDER_THREAD_NAME: &str = "actrail-live-otel-http";
const ATTR_ACTION_VALID: &str = "actrail.action.valid";
const ATTR_PROCESS_PARENT_IDENTITY_STATE: &str = "process.parent.identity_state";

struct OtelHttpOperationalMetrics {
    queue_depth: AtomicU64,
    queue_capacity: u32,
    pending_spans: AtomicU64,
    dropped_batches: AtomicU64,
    dropped_spans: AtomicU64,
    retry_attempts: AtomicU64,
    successful_batches: AtomicU64,
    partial_rejected_spans: AtomicU64,
    last_success_unix_ms: AtomicU64,
    pending_since: Mutex<Option<Instant>>,
    last_error: Mutex<Option<String>>,
}

impl OtelHttpOperationalMetrics {
    fn new(queue_capacity: u32) -> Self {
        Self {
            queue_depth: AtomicU64::new(0),
            queue_capacity,
            pending_spans: AtomicU64::new(0),
            dropped_batches: AtomicU64::new(0),
            dropped_spans: AtomicU64::new(0),
            retry_attempts: AtomicU64::new(0),
            successful_batches: AtomicU64::new(0),
            partial_rejected_spans: AtomicU64::new(0),
            last_success_unix_ms: AtomicU64::new(0),
            pending_since: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    fn queue_enter(&self) {
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
    }

    fn queue_leave(&self) {
        let _ = self
            .queue_depth
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                Some(depth.saturating_sub(1))
            });
    }

    fn set_pending(&self, spans: usize, since: Option<Instant>) {
        self.pending_spans
            .store(u64::try_from(spans).unwrap_or(u64::MAX), Ordering::Relaxed);
        if let Ok(mut slot) = self.pending_since.lock() {
            *slot = since;
        }
    }

    fn record_retry(&self) {
        self.retry_attempts.fetch_add(1, Ordering::Relaxed);
    }

    fn record_error(&self, error: impl Into<String>) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(error.into());
        }
    }

    fn record_success(&self, partial_rejected: u64) {
        self.successful_batches.fetch_add(1, Ordering::Relaxed);
        if partial_rejected > 0 {
            self.partial_rejected_spans
                .fetch_add(partial_rejected, Ordering::Relaxed);
            self.dropped_spans
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

    fn record_dropped_batch(&self, spans: usize) {
        self.dropped_batches.fetch_add(1, Ordering::Relaxed);
        self.dropped_spans
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
        let values = BTreeMap::from([
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
            dropped_records: self.dropped_spans.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|error| error.clone()),
            values,
        }
    }
}

type OtelHttpSemanticActionRoute = BestEffortSemanticActionRoute<OtelHttpSemanticActionAdapter>;

pub fn build_otel_http_observation_consumer(
    config: OtelHttpExporterConfig,
) -> Result<OtelHttpObservationConsumer, ExportError> {
    build_otel_http_observation_consumer_instance(OTEL_HTTP_BUILTIN_PLUGIN_INSTANCE_ID, config)
}

pub fn build_otel_http_observation_consumer_instance(
    instance_id: impl Into<String>,
    config: OtelHttpExporterConfig,
) -> Result<OtelHttpObservationConsumer, ExportError> {
    build_otel_http_observation_consumer_instance_with_subscriptions(
        instance_id,
        config,
        DEFAULT_OBSERVATION_EVENT_FAMILIES.to_vec(),
    )
}

pub fn build_otel_http_observation_consumer_instance_with_subscriptions(
    instance_id: impl Into<String>,
    config: OtelHttpExporterConfig,
    event_families: Vec<ObservationEventFamily>,
) -> Result<OtelHttpObservationConsumer, ExportError> {
    config
        .validate_enabled_route()
        .map_err(|message| ExportError::new(OTEL_HTTP_EXPORTER_NAME, message))?;
    let endpoint = Endpoint::parse(&config.endpoint)
        .map_err(|message| ExportError::new(OTEL_HTTP_EXPORTER_NAME, message))?;
    let metrics = Arc::new(OtelHttpOperationalMetrics::new(config.queue_capacity));
    let action_kinds = config.action_kinds.clone();
    let attribute_mode = config.attribute_mode;
    let sink = HttpBatchSink::new_with_metrics(endpoint, &config, Arc::clone(&metrics));
    let route = BestEffortSemanticActionRoute::spawn(
        OtelHttpSemanticActionAdapter {
            encoding: config.encoding,
        },
        BestEffortSemanticActionRouteConfig {
            worker_thread_name: SENDER_THREAD_NAME,
            queue_capacity: config.queue_capacity,
            shutdown_timeout: Some(config.shutdown_flush_deadline()),
        },
        sink,
    )?;
    Ok(OtelHttpObservationConsumer::new(
        instance_id,
        action_kinds,
        attribute_mode,
        route,
        event_families,
        metrics,
    ))
}

pub struct OtelHttpObservationConsumer {
    instance_id: String,
    event_families: Vec<ObservationEventFamily>,
    action_kinds: SemanticActionKindSelection,
    attribute_mode: OtelAttributeMode,
    terminal_actions: Mutex<TerminalActionLedger>,
    route: OtelHttpSemanticActionRoute,
    metrics: Arc<OtelHttpOperationalMetrics>,
}

impl OtelHttpObservationConsumer {
    fn new(
        instance_id: impl Into<String>,
        action_kinds: SemanticActionKindSelection,
        attribute_mode: OtelAttributeMode,
        route: OtelHttpSemanticActionRoute,
        event_families: Vec<ObservationEventFamily>,
        metrics: Arc<OtelHttpOperationalMetrics>,
    ) -> Self {
        Self {
            instance_id: instance_id.into(),
            event_families,
            action_kinds,
            attribute_mode,
            terminal_actions: Mutex::new(TerminalActionLedger::default()),
            route,
            metrics,
        }
    }
}

impl ObservationConsumer for OtelHttpObservationConsumer {
    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn plugin_id(&self) -> &str {
        OTEL_HTTP_PLUGIN_ID
    }

    fn runtime_kind(&self) -> PluginRuntimeKind {
        PluginRuntimeKind::Builtin
    }

    fn subscribed_event_families(&self) -> Vec<ObservationEventFamily> {
        self.event_families.clone()
    }

    fn operational_metrics_source(&self) -> Option<Arc<dyn PluginOperationalMetricsSource>> {
        Some(self.metrics.clone())
    }

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        validate_observation_batch(&batch)?;
        let mut dropped_records = Vec::new();
        let mut terminal_conflict = None;
        for action in batch.semantic_actions {
            if action.status == semantic_action::SemanticActionStatus::InProgress {
                continue;
            }
            if !self.action_kinds.enabled(action.kind) {
                continue;
            }
            let mut terminal_actions = self.terminal_actions.lock().map_err(|error| {
                PluginRuntimeError::new(
                    "otel_http_terminal_ledger",
                    format!("terminal action ledger lock poisoned: {error}"),
                )
            })?;
            if let Some(previous) =
                terminal_actions.status(action.trace_id.get(), &action.action_id)
            {
                if previous == action.status {
                    continue;
                }
                terminal_conflict = Some(PluginRuntimeError::new(
                    "otel_http_terminal_conflict",
                    format!(
                        "action {} has conflicting terminal revisions: {} then {}",
                        action.action_id,
                        previous.as_str(),
                        action.status.as_str(),
                    ),
                ));
                break;
            }
            let sanitized;
            let action = match self.attribute_mode {
                OtelAttributeMode::Full => action,
                OtelAttributeMode::MetadataOnly => {
                    sanitized = metadata_only_action(action);
                    &sanitized
                }
            };
            let record = SemanticActionExportRecord {
                trace: batch.trace,
                action,
                links: batch.semantic_links,
            };
            self.metrics.queue_enter();
            match self.route.publish(record) {
                Ok(result) => {
                    let Some(drop) = result.dropped_outcome() else {
                        terminal_actions.record(
                            action.trace_id.get(),
                            action.action_id.clone(),
                            action.status,
                        );
                        continue;
                    };
                    self.metrics.queue_leave();
                    if drop.dropped_records() == u64::default() {
                        continue;
                    }
                    dropped_records.push(PluginDroppedRecord {
                        trace_id: Some(action.trace_id),
                        plugin_instance: self.instance_id.clone(),
                        reason: drop.reason().code().to_string(),
                        queue_capacity: drop.queue_capacity(),
                        dropped_records: drop.dropped_records(),
                    });
                }
                Err(error) => {
                    self.metrics.queue_leave();
                    self.metrics
                        .record_error(format!("{}: {}", error.code, error.message));
                    dropped_records.push(PluginDroppedRecord {
                        trace_id: Some(action.trace_id),
                        plugin_instance: self.instance_id.clone(),
                        reason: format!("{}: {}", error.code, error.message),
                        queue_capacity: error.queue_capacity(),
                        dropped_records: 1,
                    });
                }
            }
        }
        if batch.trace_finalized {
            self.terminal_actions
                .lock()
                .map_err(|error| {
                    PluginRuntimeError::new(
                        "otel_http_terminal_ledger",
                        format!("terminal action ledger lock poisoned: {error}"),
                    )
                })?
                .finish_trace(batch.trace.trace_id.get());
        }
        if let Some(error) = terminal_conflict {
            return Err(error);
        }
        Ok(ObservationConsumeReport {
            dropped_records,
            reevaluate_at: None,
        })
    }

    fn finish(&self) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        let finish = self.route.finish();
        let Some(error) = finish.error() else {
            return Ok(ObservationConsumeReport::empty());
        };
        if finish.dropped_records() == u64::default() {
            return Err(PluginRuntimeError::new(
                error.code.clone(),
                error.message.clone(),
            ));
        }
        Ok(ObservationConsumeReport {
            dropped_records: vec![PluginDroppedRecord {
                trace_id: None,
                plugin_instance: self.instance_id.clone(),
                reason: format!("{}: {}", error.code, error.message),
                queue_capacity: error.queue_capacity(),
                dropped_records: finish.dropped_records(),
            }],
            reevaluate_at: None,
        })
    }
}

#[derive(Default)]
struct TerminalActionLedger {
    by_trace: HashMap<u64, HashMap<String, semantic_action::SemanticActionStatus>>,
}

impl TerminalActionLedger {
    fn status(
        &self,
        trace_id: u64,
        action_id: &str,
    ) -> Option<semantic_action::SemanticActionStatus> {
        self.by_trace
            .get(&trace_id)
            .and_then(|actions| actions.get(action_id))
            .copied()
    }

    fn record(
        &mut self,
        trace_id: u64,
        action_id: String,
        status: semantic_action::SemanticActionStatus,
    ) {
        self.by_trace
            .entry(trace_id)
            .or_default()
            .insert(action_id, status);
    }

    fn finish_trace(&mut self, trace_id: u64) {
        self.by_trace.remove(&trace_id);
    }
}

fn metadata_only_action(
    action: &semantic_action::SemanticAction,
) -> semantic_action::SemanticAction {
    let mut sanitized = action.clone();
    // Titles are often derived from command lines, paths, tool names, or LLM
    // previews. Use the stable kind as the span name at the safe boundary.
    sanitized.title = action.kind.as_str().to_string();
    sanitized.attributes.retain(|key, _| {
        matches!(
            key.as_str(),
            ATTR_ACTION_VALID | ATTR_PROCESS_PARENT_IDENTITY_STATE
        )
    });
    sanitized
}

/// One record encoded in the route's configured wire format.
enum EncodedRecord {
    /// A `{"resourceSpans":[...]}` JSON document.
    Json(String),
    /// A single `ExportTraceServiceRequest` protobuf message; concatenating
    /// several yields one valid merged request.
    Proto(Vec<u8>),
}

struct OtelHttpSemanticActionAdapter {
    encoding: OtelEncoding,
}

impl SemanticActionExportAdapter for OtelHttpSemanticActionAdapter {
    type Message = EncodedRecord;

    fn name(&self) -> &'static str {
        OTEL_HTTP_EXPORTER_NAME
    }

    fn encode(&self, record: SemanticActionExportRecord<'_>) -> Result<Self::Message, ExportError> {
        Ok(match self.encoding {
            OtelEncoding::Json => EncodedRecord::Json(export_otel_codec::render_otlp_json_line(
                record.trace,
                record.action,
                record.links,
            )),
            OtelEncoding::Protobuf => {
                EncodedRecord::Proto(export_otel_codec::render_otlp_protobuf_line(
                    record.trace,
                    record.action,
                    record.links,
                ))
            }
        })
    }
}

/// The buffered batch, in whichever wire format the route emits.
enum PendingBatch {
    /// Accumulated `resourceSpans` array elements.
    Json(Vec<serde_json::Value>),
    /// Concatenated `ExportTraceServiceRequest` bytes (`count` spans so far).
    Proto { bytes: Vec<u8>, count: usize },
}

impl PendingBatch {
    fn new(encoding: OtelEncoding) -> Self {
        match encoding {
            OtelEncoding::Json => Self::Json(Vec::new()),
            OtelEncoding::Protobuf => Self::Proto {
                bytes: Vec::new(),
                count: 0,
            },
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of buffered spans (drives the count-based flush).
    fn len(&self) -> usize {
        match self {
            Self::Json(spans) => spans.len(),
            Self::Proto { count, .. } => *count,
        }
    }

    fn clear(&mut self) {
        match self {
            Self::Json(spans) => spans.clear(),
            Self::Proto { bytes, count } => {
                bytes.clear();
                *count = 0;
            }
        }
    }

    /// Fold one encoded record into the batch. A record whose format does not
    /// match the batch, or a JSON record without `resourceSpans`, is dropped
    /// loudly rather than corrupting the batch.
    fn append(&mut self, record: EncodedRecord) {
        match (self, record) {
            (Self::Json(spans), EncodedRecord::Json(line)) => {
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(mut document) => match document
                        .get_mut("resourceSpans")
                        .and_then(serde_json::Value::as_array_mut)
                    {
                        Some(resource_spans) => spans.append(resource_spans),
                        None => {
                            eprintln!(
                                "{OTEL_HTTP_EXPORTER_NAME}: dropped record without resourceSpans"
                            )
                        }
                    },
                    Err(error) => {
                        eprintln!("{OTEL_HTTP_EXPORTER_NAME}: dropped undecodable record: {error}")
                    }
                }
            }
            (Self::Proto { bytes, count }, EncodedRecord::Proto(record_bytes)) => {
                bytes.extend_from_slice(&record_bytes);
                *count += 1;
            }
            _ => eprintln!("{OTEL_HTTP_EXPORTER_NAME}: dropped record with mismatched encoding"),
        }
    }

    /// Serialize the batch into the POST body bytes.
    fn body(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Json(spans) => {
                let body = serde_json::json!({ "resourceSpans": spans });
                serde_json::to_vec(&body).map_err(|error| error.to_string())
            }
            Self::Proto { bytes, .. } => Ok(bytes.clone()),
        }
    }
}

fn validate_observation_batch(batch: &ObservationBatch<'_>) -> Result<(), PluginRuntimeError> {
    for action in batch.semantic_actions {
        if action.trace_id != batch.trace.trace_id {
            return Err(PluginRuntimeError::new(
                OTEL_HTTP_EXPORTER_NAME,
                "semantic action trace_id does not match observation trace",
            ));
        }
    }
    for link in batch.semantic_links {
        if link.trace_id != batch.trace.trace_id {
            return Err(PluginRuntimeError::new(
                OTEL_HTTP_EXPORTER_NAME,
                "semantic action link trace_id does not match observation trace",
            ));
        }
    }
    Ok(())
}

/// Batches rendered OTLP documents and POSTs them to the collector.
///
/// Failure policy (deliberate, documented): a batch gets bounded retries with
/// backoff; when they are exhausted the batch is dropped, logged loudly, and
/// the route STAYS ALIVE — a collector outage must degrade to data loss, not
/// kill live export for the rest of the daemon's life. `finish()` (called on
/// route shutdown after the queue drains) flushes the tail under a deadline so
/// shutdown callers are never blocked indefinitely.
pub(crate) struct HttpBatchSink {
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
    connection: Option<HttpConnection>,
    dropped_batches: u64,
    metrics: Arc<OtelHttpOperationalMetrics>,
}

impl HttpBatchSink {
    #[cfg(test)]
    pub(crate) fn new(endpoint: Endpoint, config: &OtelHttpExporterConfig) -> Self {
        Self::new_with_metrics(
            endpoint,
            config,
            Arc::new(OtelHttpOperationalMetrics::new(config.queue_capacity)),
        )
    }

    fn new_with_metrics(
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
            connection: None,
            dropped_batches: 0,
            metrics,
        }
    }

    fn flush_batch(&mut self, deadline: Option<Instant>) -> Result<u64, String> {
        if self.pending.is_empty() {
            return Ok(u64::default());
        }
        let batch_spans = self.pending.len();
        let body = self
            .pending
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
                    self.pending.clear();
                    self.pending_since = None;
                    self.metrics.set_pending(0, None);
                    let durable = u64::try_from(batch_spans)
                        .unwrap_or(u64::MAX)
                        .saturating_sub(success.partial_rejected);
                    return Ok(durable);
                }
                // OTLP: 400 and other non-retryable statuses will never succeed on
                // replay — drop immediately instead of burning the retry budget.
                Err(PostError::Permanent { detail }) => {
                    last_error = detail;
                    self.metrics.record_error(last_error.clone());
                    break;
                }
                // OTLP: only 429/502/503/504 and transport errors are retried;
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
        // Exhausted or permanent: drop the batch, keep the route alive. Loud line.
        self.dropped_batches = self.dropped_batches.saturating_add(1);
        self.metrics.record_dropped_batch(batch_spans);
        eprintln!(
            "{OTEL_HTTP_EXPORTER_NAME}: dropped batch of {} spans after {} attempts \
             (total dropped batches {}): {last_error}",
            self.pending.len(),
            self.retry_max_attempts,
            self.dropped_batches,
        );
        self.pending.clear();
        self.pending_since = None;
        self.metrics.set_pending(0, None);
        Ok(u64::default())
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

/// Classification of a failed POST, per the OTLP/HTTP spec.
#[derive(Debug)]
enum PostError {
    /// Transport error or a retryable status (429/502/503/504). `retry_after`
    /// carries a server-supplied delay when present.
    Retryable {
        detail: String,
        retry_after: Option<Duration>,
    },
    /// A status that replay cannot fix (400, 401, 404, 500, ...). Drop now.
    Permanent { detail: String },
}

#[derive(Debug)]
struct PostSuccess {
    partial_rejected: u64,
}

/// OTLP/HTTP retryable status codes. Everything else non-2xx is permanent.
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

/// Parse a `Retry-After` header value in delay-seconds form (the HTTP-date form
/// is not emitted by OTLP collectors and is intentionally not handled). The
/// input is the raw header block (everything before the body).
fn parse_retry_after(header_block: &str) -> Option<Duration> {
    header_block.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("retry-after") {
            return None;
        }
        value.trim().parse::<u64>().ok().map(Duration::from_secs)
    })
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

enum HttpConnection {
    Plain(TcpStream),
    Tls(openssl::ssl::SslStream<TcpStream>),
}

impl Read for HttpConnection {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for HttpConnection {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

/// Minimal HTTP/1.1 POST over a disposable connection. Production batching
/// uses `post_otlp_reusing`; this wrapper is retained for focused TLS tests.
///
/// The HTTP framing stays hand-written on purpose — the only dependency pulled
/// in is the TLS layer, not a whole HTTP stack, to keep openEuler downstream
/// packaging review small.
#[cfg(test)]
fn post_json(
    endpoint: &Endpoint,
    body: &[u8],
    content_type: &str,
    content_encoding: Option<&str>,
    connect_timeout: Duration,
    request_timeout: Duration,
    tls: &OtelHttpTlsConfig,
) -> Result<PostSuccess, PostError> {
    let mut connection = None;
    post_otlp_reusing(
        &mut connection,
        PostRequest {
            endpoint,
            body,
            content_type,
            content_encoding,
            connect_timeout,
            request_timeout,
            tls,
        },
    )
}

struct PostRequest<'a> {
    endpoint: &'a Endpoint,
    body: &'a [u8],
    content_type: &'a str,
    content_encoding: Option<&'a str>,
    connect_timeout: Duration,
    request_timeout: Duration,
    tls: &'a OtelHttpTlsConfig,
}

fn post_otlp_reusing(
    connection: &mut Option<HttpConnection>,
    request: PostRequest<'_>,
) -> Result<PostSuccess, PostError> {
    // Transport-level failures (DNS, connect, TLS, socket I/O) are transient:
    // classify them all as retryable so a blip degrades to a retry, not a drop.
    let retryable = |detail: String| PostError::Retryable {
        detail,
        retry_after: None,
    };
    if connection.is_none() {
        *connection = Some(
            open_connection(
                request.endpoint,
                request.connect_timeout,
                request.request_timeout,
                request.tls,
            )
            .map_err(retryable)?,
        );
    }
    let response = match http_exchange(
        connection.as_mut().expect("connection initialized"),
        request.endpoint,
        request.body,
        request.content_type,
        request.content_encoding,
    ) {
        Ok(response) => response,
        Err(error) => {
            connection.take();
            return Err(retryable(error));
        }
    };
    if response.connection_close {
        connection.take();
    }
    classify_response(request.endpoint, response)
}

fn open_connection(
    endpoint: &Endpoint,
    connect_timeout: Duration,
    request_timeout: Duration,
    tls: &OtelHttpTlsConfig,
) -> Result<HttpConnection, String> {
    let addresses = endpoint
        .authority()
        .to_socket_addrs()
        .map_err(|error| format!("resolve {}: {error}", endpoint.authority()))?;
    let stream = connect_resolved_addresses(addresses, connect_timeout)
        .map_err(|error| format!("connect {}: {error}", endpoint.authority()))?;
    stream
        .set_write_timeout(Some(request_timeout))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(request_timeout))
        .map_err(|error| error.to_string())?;
    if endpoint.secure {
        tls_connect(&endpoint.host, stream, tls).map(HttpConnection::Tls)
    } else {
        Ok(HttpConnection::Plain(stream))
    }
}

fn connect_resolved_addresses(
    addresses: impl IntoIterator<Item = SocketAddr>,
    timeout: Duration,
) -> Result<TcpStream, String> {
    let started = Instant::now();
    let mut attempted = 0usize;
    let mut failures = Vec::new();
    for address in addresses {
        attempted = attempted.saturating_add(1);
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            failures.push("connect timeout exhausted".to_string());
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => failures.push(format!("{address}: {error}")),
        }
    }
    if attempted == 0 {
        Err("no resolved address".to_string())
    } else {
        Err(failures.join("; "))
    }
}

/// Map an HTTP response to the OTLP retry policy.
fn classify_response(
    endpoint: &Endpoint,
    response: HttpResponse,
) -> Result<PostSuccess, PostError> {
    let status = response.status;
    if (200..300).contains(&status) {
        // OTLP partial success: some spans rejected but the request itself is
        // accepted — must NOT be retried. Surface it loudly and move on.
        let partial_rejected = response.partial_rejected.unwrap_or_default();
        if partial_rejected > 0 {
            eprintln!(
                "{OTEL_HTTP_EXPORTER_NAME}: collector {} accepted batch with partial success, \
                 {partial_rejected} spans rejected",
                endpoint.authority()
            );
        }
        return Ok(PostSuccess { partial_rejected });
    }
    let detail = format!("collector {} returned HTTP {status}", endpoint.authority());
    if is_retryable_status(status) {
        Err(PostError::Retryable {
            detail,
            retry_after: response.retry_after,
        })
    } else {
        Err(PostError::Permanent { detail })
    }
}

/// Wrap an established TCP connection in a verified TLS session. The server
/// certificate is always verified (against `ca_cert_path`, else the system
/// trust store); a configured client cert/key is presented for mutual TLS.
fn tls_connect(
    host: &str,
    stream: TcpStream,
    tls: &OtelHttpTlsConfig,
) -> Result<openssl::ssl::SslStream<TcpStream>, String> {
    use openssl::ssl::{SslConnector, SslFiletype, SslMethod};

    let mut builder = SslConnector::builder(SslMethod::tls_client())
        .map_err(|error| format!("tls: build connector: {error}"))?;
    if let Some(ca) = tls.ca_cert_path.as_deref() {
        builder
            .set_ca_file(ca)
            .map_err(|error| format!("tls: load ca {ca}: {error}"))?;
    }
    if let (Some(cert), Some(key)) = (
        tls.client_cert_path.as_deref(),
        tls.client_key_path.as_deref(),
    ) {
        builder
            .set_certificate_chain_file(cert)
            .map_err(|error| format!("tls: load client cert {cert}: {error}"))?;
        builder
            .set_private_key_file(key, SslFiletype::PEM)
            .map_err(|error| format!("tls: load client key {key}: {error}"))?;
        builder
            .check_private_key()
            .map_err(|error| format!("tls: client key does not match cert: {error}"))?;
    }
    builder
        .build()
        .connect(host, stream)
        .map_err(|error| format!("tls: handshake with {host}: {error}"))
}

/// The parts of a collector response the retry policy needs.
struct HttpResponse {
    status: u16,
    retry_after: Option<Duration>,
    /// `rejectedSpans` from an OTLP partial-success body, when present.
    partial_rejected: Option<u64>,
    connection_close: bool,
}

/// Cap on the response bytes we buffer — status line + headers + a small OTLP
/// `ExportTraceServiceResponse` are tiny; anything larger is rejected.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Send the POST and read the response over an established stream (plain or
/// TLS). Returns the parsed status/headers/partial-success; a `String` error is
/// a transport failure (always retryable at the call site).
fn http_exchange<S: Read + Write>(
    mut stream: S,
    endpoint: &Endpoint,
    body: &[u8],
    content_type: &str,
    content_encoding: Option<&str>,
) -> Result<HttpResponse, String> {
    let content_encoding_header = content_encoding
        .map(|encoding| format!("Content-Encoding: {encoding}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: {}\r\n\
         {}Content-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        endpoint.path,
        endpoint.authority(),
        content_type,
        content_encoding_header,
        body.len(),
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .map_err(|error| format!("send to {}: {error}", endpoint.authority()))?;

    // Read until the response is complete (Content-Length aware) rather than to
    // EOF: a TLS peer that closes without `close_notify` surfaces an abrupt
    // "connection reset" instead of a clean EOF, so relying on EOF is fragile.
    let mut response = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break, // clean EOF
            Ok(read) => {
                response.extend_from_slice(&chunk[..read]);
                if response.len() >= MAX_RESPONSE_BYTES || response_is_complete(&response) {
                    break;
                }
            }
            // Some TLS peers omit close_notify. A close-delimited response can
            // still end this way, but a declared Content-Length must be complete.
            Err(error) => {
                if response_is_complete(&response) || response_is_close_delimited(&response) {
                    break;
                }
                return Err(format!(
                    "read response from {}: {error}",
                    endpoint.authority()
                ));
            }
        }
    }
    parse_http_response(&response)
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

/// Byte offset just past the `\r\n\r\n` header/body separator, if present.
fn header_block_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// A response is complete once its headers are in and the body has reached the
/// declared `Content-Length`. Without a Content-Length the length is unknown, so
/// we fall back to reading until EOF / abrupt close.
fn response_is_complete(buf: &[u8]) -> bool {
    let Some(body_start) = header_block_end(buf) else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..body_start]);
    if response_is_chunked(&head) {
        return decode_chunked_body(&buf[body_start..]).is_ok();
    }
    match content_length(&head) {
        Some(len) => buf.len() - body_start >= len,
        None => false,
    }
}

fn response_is_close_delimited(buf: &[u8]) -> bool {
    let Some(body_start) = header_block_end(buf) else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..body_start]);
    content_length(&head).is_none() && !response_is_chunked(&head)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    })
}

fn parse_http_response(response: &[u8]) -> Result<HttpResponse, String> {
    let body_start = header_block_end(response)
        .ok_or_else(|| "malformed HTTP response: missing header terminator".to_string())?;
    let head = std::str::from_utf8(&response[..body_start - 4])
        .map_err(|error| format!("malformed HTTP response headers: {error}"))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(version), Some(code)) if version.starts_with("HTTP/") => {
                    code.parse::<u16>().ok()
                }
                _ => None,
            }
        })
        .ok_or_else(|| "malformed HTTP response".to_string())?;
    let available_body = &response[body_start..];
    let decoded_chunked;
    let body = if response_is_chunked(head) {
        decoded_chunked = decode_chunked_body(available_body)?;
        decoded_chunked.as_slice()
    } else {
        match content_length(head) {
            Some(expected) if available_body.len() < expected => {
                return Err(format!(
                    "truncated HTTP response body: expected {expected} bytes, received {}",
                    available_body.len()
                ));
            }
            Some(expected) => &available_body[..expected],
            None => available_body,
        }
    };
    let partial_rejected = if (200..300).contains(&status) {
        parse_partial_rejected_response(head, body)?
    } else {
        None
    };
    Ok(HttpResponse {
        status,
        retry_after: parse_retry_after(head),
        partial_rejected,
        connection_close: response_connection_must_close(head),
    })
}

fn response_connection_must_close(head: &str) -> bool {
    let connection_close = header_value(head, "connection").is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("close"))
    });
    let connection_keep_alive = header_value(head, "connection").is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("keep-alive"))
    });
    let http_11_or_newer = head
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("HTTP/1.1") || line.starts_with("HTTP/2"));
    connection_close
        || (!http_11_or_newer && !connection_keep_alive)
        || (content_length(head).is_none() && !response_is_chunked(head))
}

fn response_is_chunked(head: &str) -> bool {
    header_value(head, "transfer-encoding").is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("chunked"))
    })
}

fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut offset = 0usize;
    let mut decoded = Vec::new();
    loop {
        let size_line_end = body[offset..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "truncated chunked HTTP response size".to_string())?;
        let size_line = std::str::from_utf8(&body[offset..offset + size_line_end])
            .map_err(|error| format!("invalid chunk size line: {error}"))?;
        let size_token = size_line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_token, 16)
            .map_err(|error| format!("invalid chunk size {size_token:?}: {error}"))?;
        offset = offset.saturating_add(size_line_end + 2);
        if size == 0 {
            if body.get(offset..offset + 2) == Some(b"\r\n") {
                return Ok(decoded);
            }
            body[offset..]
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .ok_or_else(|| "truncated chunked HTTP response trailers".to_string())?;
            return Ok(decoded);
        }
        let data_end = offset
            .checked_add(size)
            .ok_or_else(|| "chunked HTTP response size overflow".to_string())?;
        let framed_end = data_end
            .checked_add(2)
            .ok_or_else(|| "chunked HTTP response size overflow".to_string())?;
        if body.len() < framed_end {
            return Err(format!(
                "truncated chunked HTTP response body: expected {size} byte chunk"
            ));
        }
        if body.get(data_end..framed_end) != Some(b"\r\n") {
            return Err("invalid chunked HTTP response delimiter".to_string());
        }
        decoded.extend_from_slice(&body[offset..data_end]);
        offset = framed_end;
    }
}

fn parse_partial_rejected_response(head: &str, body: &[u8]) -> Result<Option<u64>, String> {
    if body.is_empty() {
        return Ok(None);
    }
    match header_value(head, "content-type")
        .and_then(|value| value.split(';').next())
        .map(str::trim)
    {
        Some(value) if value.eq_ignore_ascii_case("application/x-protobuf") => {
            export_otel_codec::parse_otlp_protobuf_partial_rejected(body)
        }
        Some(value) if value.eq_ignore_ascii_case("application/json") => {
            let body = std::str::from_utf8(body)
                .map_err(|error| format!("decode OTLP JSON response as UTF-8: {error}"))?;
            let value = serde_json::from_str::<serde_json::Value>(body)
                .map_err(|error| format!("decode OTLP JSON response: {error}"))?;
            Ok(partial_rejected_from_value(&value))
        }
        Some(value) => Err(format!("unsupported OTLP response Content-Type {value:?}")),
        None => {
            let Ok(body) = std::str::from_utf8(body) else {
                return Err("OTLP response body has no Content-Type and is not UTF-8".to_string());
            };
            Ok(parse_partial_rejected(body))
        }
    }
}

fn header_value<'a>(head: &'a str, expected_name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(expected_name)
            .then_some(value.trim())
    })
}

/// Extract `partialSuccess.rejectedSpans` from an OTLP response body, tolerating
/// both the JSON string and number encodings. Absent/garbage => `None`.
fn parse_partial_rejected(body: &str) -> Option<u64> {
    let value = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
    partial_rejected_from_value(&value)
}

fn partial_rejected_from_value(value: &serde_json::Value) -> Option<u64> {
    let rejected = value.get("partialSuccess")?.get("rejectedSpans")?;
    rejected
        .as_u64()
        .or_else(|| rejected.as_str().and_then(|s| s.parse::<u64>().ok()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use super::{
        Endpoint, HttpBatchSink, OtelCompression, OtelHttpExporterConfig,
        build_otel_http_observation_consumer,
    };
    use export_core::{BestEffortDelivery, BestEffortDeliveryConfig, BestEffortSink};
    use model_core::ids::{ProfileName, TraceId, TraceName};
    use model_core::process::ProcessIdentity;
    use model_core::trace::{TraceAlertToken, TraceLifecycleState, TraceRecord};
    use plugin_system::{ObservationBatch, ObservationConsumer, PluginOperationalMetricsSource};
    use semantic_action::{
        SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    };

    fn test_config(endpoint: String) -> OtelHttpExporterConfig {
        OtelHttpExporterConfig {
            endpoint,
            allow_insecure: true,
            queue_capacity: 16,
            batch_max_spans: 2,
            batch_timeout_ms: 60_000, // large: count-based tests must not race the timer
            connect_timeout_ms: 1000,
            request_timeout_ms: 1000,
            retry_max_attempts: 2,
            retry_backoff_ms: 10,
            shutdown_flush_deadline_ms: 500,
            tls: crate::config::OtelHttpTlsConfig::default(),
            encoding: crate::config::OtelEncoding::Json,
            compression: crate::config::OtelCompression::None,
            action_kinds: export_core::SemanticActionKindSelection::from_config_entries([
                ("default".to_string(), false),
                ("process.exec".to_string(), true),
                ("llm.response".to_string(), true),
            ])
            .expect("test action policy"),
            attribute_mode: crate::config::OtelAttributeMode::MetadataOnly,
        }
    }

    /// One-shot stub collector: accepts a connection, captures the request,
    /// answers 200, and hands the request body back through the channel.
    fn spawn_stub_collector(responses: usize) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub collector");
        let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..responses {
                let Ok((mut socket, _)) = listener.accept() else {
                    return;
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                // Connection: close => read until EOF or the socket times out.
                while let Ok(read) = socket.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request_complete(&request) {
                        break;
                    }
                }
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                let _ = sender.send(String::from_utf8_lossy(&request).to_string());
            }
        });
        (endpoint, receiver)
    }

    fn spawn_blackhole_collector() -> (String, mpsc::Receiver<()>, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind blackhole collector");
        let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed_attempts = Arc::clone(&attempts);
        std::thread::spawn(move || {
            let Ok((socket, _)) = listener.accept() else {
                return;
            };
            observed_attempts.fetch_add(1, Ordering::Relaxed);
            let _ = sender.send(());
            std::thread::sleep(Duration::from_millis(200));
            drop(socket);

            listener
                .set_nonblocking(true)
                .expect("set blackhole listener nonblocking");
            let observe_until = Instant::now() + Duration::from_millis(400);
            while Instant::now() < observe_until {
                match listener.accept() {
                    Ok((socket, _)) => {
                        observed_attempts.fetch_add(1, Ordering::Relaxed);
                        drop(socket);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return,
                }
            }
        });
        (endpoint, receiver, attempts)
    }

    fn request_complete(request: &[u8]) -> bool {
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().parse::<usize>().unwrap_or(0))
            })
            .unwrap_or(0);
        request.len() >= header_end + 4 + content_length
    }

    #[test]
    fn consumer_filters_action_kinds_and_keeps_content_attributes_local_by_default() {
        let (endpoint, received) = spawn_stub_collector(1);
        let mut config = test_config(endpoint);
        config.batch_timeout_ms = 20;
        config.action_kinds = export_core::SemanticActionKindSelection::from_config_entries([
            ("default".to_string(), false),
            ("process.exec".to_string(), true),
        ])
        .expect("explicit action selection");
        let consumer = build_otel_http_observation_consumer(config).expect("build consumer");
        let trace = TraceRecord::new(
            TraceId::new(7),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(100),
            TraceName::new("otel-http-policy"),
            ProfileName::new("test"),
            UNIX_EPOCH,
        );
        let mut process_action = semantic_action(
            "process",
            SemanticActionKind::ProcessExec,
            BTreeMap::from([(
                "command.line".to_string(),
                "super-secret --token value".to_string(),
            )]),
        );
        process_action.title = "super-secret action title".to_string();
        let actions = vec![
            process_action,
            semantic_action(
                "llm",
                SemanticActionKind::LlmRequest,
                BTreeMap::from([(
                    "llm.request.message_preview".to_string(),
                    "must-not-leave".to_string(),
                )]),
            ),
        ];

        consumer
            .consume(ObservationBatch {
                trace: &trace,
                trace_finalized: false,
                semantic_actions: &actions,
                semantic_links: &[],
                file_observation_paths: &[],
                payload_segments: &[],
            })
            .expect("consume observations");

        let request = received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector receives selected terminal action");
        assert!(request.contains("process.exec"));
        assert!(!request.contains("llm.request"));
        assert!(!request.contains("super-secret"));
        assert!(!request.contains("super-secret action title"));
        assert!(!request.contains("must-not-leave"));
    }

    #[test]
    fn consumer_emits_only_the_terminal_revision_of_an_action() {
        let (endpoint, received) = spawn_stub_collector(1);
        let mut config = test_config(endpoint);
        config.batch_timeout_ms = 20;
        let consumer = build_otel_http_observation_consumer(config).expect("build consumer");
        let trace = TraceRecord::new(
            TraceId::new(7),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(100),
            TraceName::new("otel-http-terminal"),
            ProfileName::new("test"),
            UNIX_EPOCH,
        );
        let mut in_progress = semantic_action(
            "same-action",
            SemanticActionKind::LlmResponse,
            BTreeMap::new(),
        );
        in_progress.status = SemanticActionStatus::InProgress;
        in_progress.end_time = None;
        let terminal = semantic_action(
            "same-action",
            SemanticActionKind::LlmResponse,
            BTreeMap::new(),
        );
        let duplicate_terminal = terminal.clone();

        for action in [&in_progress, &terminal, &duplicate_terminal] {
            consumer
                .consume(ObservationBatch {
                    trace: &trace,
                    trace_finalized: false,
                    semantic_actions: std::slice::from_ref(action),
                    semantic_links: &[],
                    file_observation_paths: &[],
                    payload_segments: &[],
                })
                .expect("consume action revision");
        }

        let request = received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector receives terminal action");
        assert_eq!(request.matches("\"actrail.action.id\"").count(), 1);
        assert!(request.contains("success"));
        assert!(!request.contains("in_progress"));
    }

    #[test]
    fn consumer_rejects_conflicting_terminal_revisions_and_releases_final_ledger() {
        let (endpoint, received) = spawn_stub_collector(1);
        let mut config = test_config(endpoint);
        config.batch_timeout_ms = 20;
        let consumer = build_otel_http_observation_consumer(config).expect("build consumer");
        let trace = TraceRecord::new(
            TraceId::new(7),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(100),
            TraceName::new("otel-http-terminal-conflict"),
            ProfileName::new("test"),
            UNIX_EPOCH,
        );
        let success = semantic_action(
            "same-action",
            SemanticActionKind::LlmResponse,
            BTreeMap::new(),
        );
        let mut conflicting = success.clone();
        conflicting.status = SemanticActionStatus::Error;

        consumer
            .consume(ObservationBatch {
                trace: &trace,
                trace_finalized: false,
                semantic_actions: std::slice::from_ref(&success),
                semantic_links: &[],
                file_observation_paths: &[],
                payload_segments: &[],
            })
            .expect("queue first terminal revision");
        let error = consumer
            .consume(ObservationBatch {
                trace: &trace,
                trace_finalized: true,
                semantic_actions: std::slice::from_ref(&conflicting),
                semantic_links: &[],
                file_observation_paths: &[],
                payload_segments: &[],
            })
            .expect_err("conflicting terminal revision must be surfaced");
        assert_eq!(error.code, "otel_http_terminal_conflict");
        assert!(
            consumer
                .terminal_actions
                .lock()
                .expect("terminal ledger lock")
                .by_trace
                .is_empty(),
            "a conflicting final batch must still release trace-scoped state"
        );

        let request = received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector receives only the first terminal revision");
        assert_eq!(request.matches("\"actrail.action.id\"").count(), 1);
        assert!(request.contains("success"));
        assert!(!request.contains("error"));
    }

    #[test]
    fn consumer_releases_terminal_ledger_only_with_the_final_projection_batch() {
        let (endpoint, received) = spawn_stub_collector(1);
        let mut config = test_config(endpoint);
        config.batch_max_spans = 1;
        let consumer = build_otel_http_observation_consumer(config).expect("build consumer");
        let mut trace = TraceRecord::new(
            TraceId::new(7),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(100),
            TraceName::new("otel-http-terminal-ledger"),
            ProfileName::new("test"),
            UNIX_EPOCH,
        );
        trace.lifecycle_state = TraceLifecycleState::Exited;
        let action = semantic_action(
            "terminal-trace-action",
            SemanticActionKind::ProcessExec,
            BTreeMap::new(),
        );

        consumer
            .consume(ObservationBatch {
                trace: &trace,
                trace_finalized: false,
                semantic_actions: std::slice::from_ref(&action),
                semantic_links: &[],
                file_observation_paths: &[],
                payload_segments: &[],
            })
            .expect("consume terminal trace action");

        assert_eq!(
            consumer
                .terminal_actions
                .lock()
                .expect("terminal ledger lock")
                .by_trace
                .len(),
            1,
            "a terminal lifecycle state alone is not the final export boundary"
        );
        consumer
            .consume(ObservationBatch {
                trace: &trace,
                trace_finalized: false,
                semantic_actions: std::slice::from_ref(&action),
                semantic_links: &[],
                file_observation_paths: &[],
                payload_segments: &[],
            })
            .expect("suppress a later duplicate terminal revision");
        consumer
            .consume(ObservationBatch {
                trace: &trace,
                trace_finalized: true,
                semantic_actions: &[],
                semantic_links: &[],
                file_observation_paths: &[],
                payload_segments: &[],
            })
            .expect("consume the final projection boundary");

        assert!(
            consumer
                .terminal_actions
                .lock()
                .expect("terminal ledger lock")
                .by_trace
                .is_empty(),
            "the final projection batch must release its trace ledger"
        );
        received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector receives terminal trace action");
    }

    #[test]
    fn consumer_finish_reports_drop_and_returns_by_shutdown_deadline() {
        let (endpoint, connected, attempts) = spawn_blackhole_collector();
        let mut config = test_config(endpoint);
        config.batch_max_spans = 1;
        config.retry_max_attempts = 3;
        config.request_timeout_ms = 1000;
        config.shutdown_flush_deadline_ms = 50;
        let consumer = build_otel_http_observation_consumer(config).expect("build consumer");
        let trace = TraceRecord::new(
            TraceId::new(7),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(100),
            TraceName::new("otel-http-deadline"),
            ProfileName::new("test"),
            UNIX_EPOCH,
        );
        let action = semantic_action(
            "blocked-action",
            SemanticActionKind::ProcessExec,
            BTreeMap::new(),
        );
        consumer
            .consume(ObservationBatch {
                trace: &trace,
                trace_finalized: false,
                semantic_actions: std::slice::from_ref(&action),
                semantic_links: &[],
                file_observation_paths: &[],
                payload_segments: &[],
            })
            .expect("queue terminal action");
        connected
            .recv_timeout(Duration::from_secs(1))
            .expect("delivery is blocked in collector request");

        let started = std::time::Instant::now();
        let report = consumer.finish().expect("finish returns a drop report");

        assert!(started.elapsed() < Duration::from_millis(200));
        assert_eq!(
            report
                .dropped_records
                .iter()
                .map(|drop| drop.dropped_records)
                .sum::<u64>(),
            1
        );
        assert!(
            report.dropped_records[0]
                .reason
                .contains("shutdown deadline exceeded")
        );
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            attempts.load(Ordering::Relaxed),
            1,
            "shared shutdown deadline must stop retries after blocked I/O returns"
        );
    }

    fn semantic_action(
        id: &str,
        kind: SemanticActionKind,
        attributes: BTreeMap<String, String>,
    ) -> SemanticAction {
        SemanticAction {
            action_id: id.to_string(),
            trace_id: TraceId::new(7),
            kind,
            title: id.to_string(),
            start_time: UNIX_EPOCH,
            end_time: Some(UNIX_EPOCH + Duration::from_millis(1)),
            process: ProcessIdentity::new(100),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            confidence_millis: None,
            attributes,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn connect_falls_back_to_later_resolved_address() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fallback target");
        let good = listener.local_addr().expect("target address");
        let unavailable = "127.0.0.1:0".parse().expect("unavailable address");

        let stream = super::connect_resolved_addresses([unavailable, good], Duration::from_secs(1))
            .expect("second address connects");

        assert_eq!(stream.peer_addr().expect("peer address"), good);
    }

    #[test]
    fn gzip_request_body_round_trips() {
        let original = b"{\"resourceSpans\":[{\"scopeSpans\":[]}]}";
        let compressed =
            super::encode_request_body(original, OtelCompression::Gzip).expect("gzip request body");
        let mut decoder = flate2::read::GzDecoder::new(compressed.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).expect("decode gzip body");

        assert_eq!(decoded, original);
    }

    #[test]
    fn gzip_delivery_sets_content_encoding_header() {
        let (endpoint, received) = spawn_stub_collector(1);
        let mut config = test_config(endpoint.clone());
        config.compression = OtelCompression::Gzip;
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("first")).expect("buffered");
        sink.deliver(span_line("second")).expect("batch flushed");

        let request = received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector received compressed batch");
        assert!(request.contains("Content-Encoding: gzip\r\n"));
    }

    #[test]
    fn consecutive_batches_reuse_one_http_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind keepalive collector");
        let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
        let (sender, received) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept one connection");
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            for _ in 0..2 {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !request_complete(&request) {
                    let read = socket.read(&mut buffer).expect("read keepalive request");
                    assert!(read > 0, "connection closed before second request");
                    request.extend_from_slice(&buffer[..read]);
                }
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .expect("write keepalive response");
                sender
                    .send(String::from_utf8_lossy(&request).to_string())
                    .unwrap();
            }
        });
        let mut config = test_config(endpoint.clone());
        config.batch_max_spans = 1;
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("first")).expect("first batch");
        sink.deliver(span_line("second")).expect("second batch");

        let first = received
            .recv_timeout(Duration::from_secs(2))
            .expect("first request");
        let second = received
            .recv_timeout(Duration::from_secs(2))
            .expect("second request");
        assert!(first.contains("Connection: keep-alive"));
        assert!(second.contains("second"));
        assert_eq!(sink.dropped_batches, 0);
    }

    fn span_line(name: &str) -> super::EncodedRecord {
        super::EncodedRecord::Json(format!(
            "{{\"resourceSpans\":[{{\"resource\":{{\"attributes\":[]}},\
             \"scopeSpans\":[{{\"spans\":[{{\"name\":\"{name}\"}}]}}]}}]}}"
        ))
    }

    #[test]
    fn batches_and_posts_spans_to_collector() {
        let (endpoint, received) = spawn_stub_collector(1);
        let config = test_config(endpoint.clone());
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("first")).expect("buffered");
        sink.deliver(span_line("second")).expect("batch flushed");

        let request = received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector received batch");
        assert!(request.starts_with("POST /v1/traces HTTP/1.1"));
        assert!(request.contains("\"first\""));
        assert!(request.contains("\"second\""));
        assert!(sink.pending.is_empty());
    }

    #[test]
    fn operational_metrics_track_pending_batch_and_queue_depth() {
        let (endpoint, _received) = spawn_stub_collector(1);
        let config = test_config(endpoint.clone());
        let metrics = Arc::new(super::OtelHttpOperationalMetrics::new(
            config.queue_capacity,
        ));
        let mut sink = HttpBatchSink::new_with_metrics(
            Endpoint::parse(&endpoint).unwrap(),
            &config,
            Arc::clone(&metrics),
        );

        metrics.queue_enter();
        sink.deliver(span_line("pending")).expect("buffered");

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.queue_depth, Some(0));
        assert_eq!(snapshot.queue_capacity, Some(config.queue_capacity));
        assert_eq!(snapshot.values["otel_http.pending_spans"], 1);
        assert_eq!(snapshot.values["otel_http.successful_batches"], 0);
    }

    #[test]
    fn finish_flushes_tail_under_deadline() {
        let (endpoint, received) = spawn_stub_collector(1);
        let config = test_config(endpoint.clone());
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("tail")).expect("buffered only");
        sink.finish().expect("flush on shutdown");

        let request = received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector received tail");
        assert!(request.contains("\"tail\""));
    }

    #[test]
    fn later_delivery_flushes_an_aged_partial_batch() {
        // Deliver itself also checks age, so a late record flushes immediately
        // even before the worker's next idle wakeup.
        let (endpoint, received) = spawn_stub_collector(1);
        let mut config = test_config(endpoint.clone());
        config.batch_max_spans = 100; // count trigger unreachable in this test
        config.batch_timeout_ms = 100; // age threshold for the next delivery
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("early"))
            .expect("buffered, not full");
        assert_eq!(sink.pending.len(), 1, "still buffered before timeout");
        std::thread::sleep(Duration::from_millis(150)); // exceed batch_timeout
        sink.deliver(span_line("later"))
            .expect("second delivery should flush the aged partial batch");

        let request = received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector received an age-triggered flush");
        assert!(request.contains("\"early\""), "buffered span was flushed");
        assert!(sink.pending.is_empty(), "pending drained after timed flush");
    }

    #[test]
    fn delivery_worker_flushes_partial_batch_while_input_is_idle() {
        let (endpoint, received) = spawn_stub_collector(1);
        let mut config = test_config(endpoint.clone());
        config.batch_max_spans = 100;
        config.batch_timeout_ms = 100;
        let sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);
        let delivery = BestEffortDelivery::spawn(
            BestEffortDeliveryConfig {
                component_name: "otel-http-idle-test",
                worker_thread_name: "otel-http-idle-test-worker",
                queue_capacity: 8,
                shutdown_timeout: None,
            },
            sink,
        )
        .expect("spawn delivery worker");

        delivery
            .publish(span_line("idle-tail"))
            .expect("queue one span");

        let request = received
            .recv_timeout(Duration::from_secs(1))
            .expect("collector receives the partial batch without a second record");
        assert!(request.contains("idle-tail"));
    }

    #[test]
    fn collector_outage_drops_batch_but_keeps_sink_alive() {
        // Unroutable endpoint: connect fails fast, retries exhaust, batch drops.
        let endpoint = "http://127.0.0.1:9/v1/traces".to_string();
        let config = test_config(endpoint.clone());
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("a")).expect("buffered");
        sink.deliver(span_line("b")).expect("dropped, not fatal");
        assert_eq!(sink.dropped_batches, 1);
        assert!(sink.pending.is_empty());

        // Sink still accepts and flushes new work.
        sink.deliver(span_line("c")).expect("still alive");
        assert_eq!(sink.pending.len(), 1);
    }

    // ---- OTLP-compliant retry policy (G6) ----

    /// Collector that answers each connection with the next scripted raw HTTP
    /// response, and reports every request it received through the channel.
    fn spawn_scripted_collector(responses: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind scripted collector");
        let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for reply in responses {
                let Ok((mut socket, _)) = listener.accept() else {
                    return;
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while let Ok(read) = socket.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request_complete(&request) {
                        break;
                    }
                }
                let _ = socket.write_all(reply.as_bytes());
                let _ = sender.send(String::from_utf8_lossy(&request).to_string());
            }
        });
        (endpoint, receiver)
    }

    fn status_reply(status: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    }

    #[test]
    fn retries_on_503_then_succeeds() {
        // 503 is retryable: first attempt fails, second delivers.
        let (endpoint, received) = spawn_scripted_collector(vec![
            status_reply("503 Service Unavailable"),
            status_reply("200 OK"),
        ]);
        let config = test_config(endpoint.clone());
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("x")).expect("buffered");
        sink.deliver(span_line("y"))
            .expect("batch flushed after retry");

        assert!(
            received.recv_timeout(Duration::from_secs(2)).is_ok(),
            "first (503) attempt"
        );
        assert!(
            received.recv_timeout(Duration::from_secs(2)).is_ok(),
            "second (200) attempt"
        );
        assert_eq!(
            sink.dropped_batches, 0,
            "delivered on retry, nothing dropped"
        );
        assert!(sink.pending.is_empty());
        let metrics = sink.metrics.snapshot();
        assert_eq!(metrics.values["otel_http.retry_attempts"], 1);
        assert_eq!(metrics.values["otel_http.successful_batches"], 1);
        assert_eq!(metrics.values["otel_http.pending_spans"], 0);
        assert_eq!(metrics.dropped_records, 0);
        assert_eq!(metrics.last_error, None);
    }

    #[test]
    fn does_not_retry_on_400() {
        // 400 is permanent: drop immediately, do NOT burn the retry budget.
        let (endpoint, received) = spawn_scripted_collector(vec![
            status_reply("400 Bad Request"),
            status_reply("200 OK"),
        ]);
        let config = test_config(endpoint.clone());
        assert!(
            config.retry_max_attempts >= 2,
            "test needs a retry budget to prove it is unused"
        );
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("x")).expect("buffered");
        sink.deliver(span_line("y"))
            .expect("batch dropped, not fatal");

        assert!(
            received.recv_timeout(Duration::from_secs(2)).is_ok(),
            "exactly one request sent"
        );
        assert!(
            received.recv_timeout(Duration::from_millis(300)).is_err(),
            "400 must not be retried — no second request"
        );
        assert_eq!(sink.dropped_batches, 1, "permanent failure drops the batch");
        let metrics = sink.metrics.snapshot();
        assert_eq!(metrics.values["otel_http.dropped_batches"], 1);
        assert_eq!(metrics.dropped_records, 2);
        assert!(metrics.last_error.is_some());
    }

    #[test]
    fn partial_success_is_counted_as_rejected_spans_without_retry() {
        let body = r#"{"partialSuccess":{"rejectedSpans":"1"}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (endpoint, received) = spawn_scripted_collector(vec![response]);
        let config = test_config(endpoint.clone());
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("first")).expect("buffered");
        sink.deliver(span_line("second"))
            .expect("partially accepted batch");

        assert!(received.recv_timeout(Duration::from_secs(2)).is_ok());
        let metrics = sink.metrics.snapshot();
        assert_eq!(metrics.values["otel_http.partial_rejected_spans"], 1);
        assert_eq!(metrics.values["otel_http.retry_attempts"], 0);
        assert_eq!(metrics.values["otel_http.successful_batches"], 1);
        assert_eq!(metrics.dropped_records, 1);
        assert_eq!(metrics.last_error, None);
    }

    #[test]
    fn retries_429_honoring_retry_after() {
        // 429 with Retry-After: 0 is retryable and must succeed on the next try.
        let mut retry_after = status_reply("429 Too Many Requests");
        retry_after = retry_after.replace(
            "Content-Length: 0\r\n\r\n",
            "Retry-After: 0\r\nContent-Length: 0\r\n\r\n",
        );
        let (endpoint, received) =
            spawn_scripted_collector(vec![retry_after, status_reply("200 OK")]);
        let config = test_config(endpoint.clone());
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(span_line("x")).expect("buffered");
        sink.deliver(span_line("y"))
            .expect("delivered after honoring Retry-After");

        assert!(
            received.recv_timeout(Duration::from_secs(2)).is_ok(),
            "429 attempt"
        );
        assert!(
            received.recv_timeout(Duration::from_secs(2)).is_ok(),
            "retried attempt"
        );
        assert_eq!(sink.dropped_batches, 0);
    }

    #[test]
    fn parse_retry_after_reads_delay_seconds() {
        let head = "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 7\r\nContent-Length: 0";
        assert_eq!(super::parse_retry_after(head), Some(Duration::from_secs(7)));
        // Case-insensitive header name; absent header => None.
        let head2 = "HTTP/1.1 503\r\nretry-after:  2 \r\n";
        assert_eq!(
            super::parse_retry_after(head2),
            Some(Duration::from_secs(2))
        );
        assert_eq!(super::parse_retry_after("HTTP/1.1 200 OK\r\n"), None);
    }

    #[test]
    fn only_429_502_503_504_are_retryable() {
        for status in [429u16, 502, 503, 504] {
            assert!(super::is_retryable_status(status), "{status} should retry");
        }
        for status in [400u16, 401, 403, 404, 500, 501] {
            assert!(
                !super::is_retryable_status(status),
                "{status} should not retry"
            );
        }
    }

    // ---- protobuf encoding wiring ----

    #[test]
    fn protobuf_encoding_posts_concatenated_bytes_with_protobuf_content_type() {
        // The zero-loss correctness of the protobuf bytes is proven in the codec;
        // here we prove the transport wiring: protobuf content-type + records
        // batched by byte concatenation into one POST.
        let (endpoint, received) = spawn_stub_collector(1);
        let mut config = test_config(endpoint.clone());
        config.encoding = crate::config::OtelEncoding::Protobuf; // batch_max_spans = 2
        let mut sink = HttpBatchSink::new(Endpoint::parse(&endpoint).unwrap(), &config);

        sink.deliver(super::EncodedRecord::Proto(b"\x0a\x03one".to_vec()))
            .expect("buffered");
        sink.deliver(super::EncodedRecord::Proto(b"\x0a\x03two".to_vec()))
            .expect("batch flushed at count 2");

        let request = received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector received protobuf batch");
        assert!(
            request.contains("Content-Type: application/x-protobuf"),
            "protobuf encoding must set the protobuf content type: {request}"
        );
        // Both records reached the collector, concatenated in one body.
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        assert!(
            body.contains("one") && body.contains("two"),
            "both records concatenated"
        );
        assert!(sink.pending.is_empty());
    }

    #[test]
    fn partial_success_rejected_span_count_is_parsed() {
        assert_eq!(
            super::parse_partial_rejected("{\"partialSuccess\":{\"rejectedSpans\":\"5\"}}"),
            Some(5)
        );
        assert_eq!(
            super::parse_partial_rejected("{\"partialSuccess\":{\"rejectedSpans\":3}}"),
            Some(3)
        );
        assert_eq!(super::parse_partial_rejected("{}"), None);
        assert_eq!(super::parse_partial_rejected(""), None);
    }

    #[test]
    fn protobuf_partial_success_rejected_span_count_is_parsed() {
        // ExportTraceServiceResponse {
        //   partial_success: ExportTracePartialSuccess { rejected_spans: 5 }
        // }
        let body = [0x0a, 0x02, 0x08, 0x05];
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 4\r\n\r\n"
                .to_vec();
        response.extend_from_slice(&body);

        let parsed = super::parse_http_response(&response).expect("valid protobuf response");
        assert_eq!(parsed.partial_rejected, Some(5));
    }

    #[test]
    fn chunked_protobuf_partial_success_is_parsed_and_reusable() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n\x0a\x02\x08\x05\r\n0\r\n\r\n";

        let parsed = super::parse_http_response(response).expect("valid chunked response");
        assert_eq!(parsed.partial_rejected, Some(5));
        assert!(!parsed.connection_close);
    }

    #[test]
    fn http_10_response_without_keep_alive_is_not_reused() {
        let response = b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n";

        let parsed = super::parse_http_response(response).expect("valid HTTP/1.0 response");
        assert!(parsed.connection_close);
    }

    #[test]
    fn response_with_truncated_declared_body_is_rejected() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 4\r\n\r\n\x0a\x02";

        let error = match super::parse_http_response(response) {
            Ok(_) => panic!("truncated body was accepted"),
            Err(error) => error,
        };
        assert!(error.contains("truncated"), "unexpected error: {error}");
    }

    #[test]
    fn malformed_json_success_response_is_rejected() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\n\r\n{";

        let error = match super::parse_http_response(response) {
            Ok(_) => panic!("malformed JSON success response was accepted"),
            Err(error) => error,
        };
        assert!(error.contains("decode OTLP JSON response"));
    }

    #[test]
    fn backoff_with_jitter_stays_within_bounds() {
        let base = Duration::from_millis(100);
        for attempt in 0..6u32 {
            let ceiling = base
                .saturating_mul(1u32 << attempt.min(16))
                .min(Duration::from_secs(30));
            for _ in 0..50 {
                let wait = super::backoff_with_jitter(base, attempt);
                assert!(wait <= ceiling, "wait {wait:?} exceeds ceiling {ceiling:?}");
                assert!(
                    wait >= ceiling / 2,
                    "wait {wait:?} below half of {ceiling:?}"
                );
            }
        }
    }
}

/// Mutual-TLS transport tests: a real OpenSSL server that demands a client
/// certificate, exercised end-to-end through `post_json`.
#[cfg(test)]
mod mtls_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::Duration;

    use openssl::asn1::Asn1Time;
    use openssl::bn::{BigNum, MsbOption};
    use openssl::hash::MessageDigest;
    use openssl::pkey::{PKey, Private};
    use openssl::rsa::Rsa;
    use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod, SslVerifyMode};
    use openssl::x509::extension::{BasicConstraints, KeyUsage, SubjectAlternativeName};
    use openssl::x509::{X509, X509NameBuilder};

    use super::{Endpoint, post_json};
    use crate::config::OtelHttpTlsConfig;

    struct Ca {
        cert: X509,
        key: PKey<Private>,
    }

    fn keypair() -> PKey<Private> {
        PKey::from_rsa(Rsa::generate(2048).expect("rsa")).expect("pkey")
    }

    fn name(cn: &str) -> openssl::x509::X509Name {
        let mut b = X509NameBuilder::new().expect("name builder");
        b.append_entry_by_text("CN", cn).expect("cn");
        b.build()
    }

    fn serial() -> openssl::asn1::Asn1Integer {
        let mut bn = BigNum::new().expect("bn");
        bn.rand(159, MsbOption::MAYBE_ZERO, false).expect("rand");
        bn.to_asn1_integer().expect("serial")
    }

    fn mk_ca() -> Ca {
        let key = keypair();
        let mut b = X509::builder().expect("builder");
        b.set_version(2).unwrap();
        b.set_serial_number(&serial()).unwrap();
        b.set_subject_name(&name("actrail-test-ca")).unwrap();
        b.set_issuer_name(&name("actrail-test-ca")).unwrap();
        b.set_pubkey(&key).unwrap();
        b.set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        b.set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        b.append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        b.sign(&key, MessageDigest::sha256()).unwrap();
        Ca {
            cert: b.build(),
            key,
        }
    }

    /// Leaf cert signed by `ca`. `server` adds a localhost SAN so the client's
    /// hostname verification passes.
    fn mk_leaf(ca: &Ca, cn: &str, server: bool) -> (X509, PKey<Private>) {
        let key = keypair();
        let mut b = X509::builder().expect("builder");
        b.set_version(2).unwrap();
        b.set_serial_number(&serial()).unwrap();
        b.set_subject_name(&name(cn)).unwrap();
        b.set_issuer_name(ca.cert.subject_name()).unwrap();
        b.set_pubkey(&key).unwrap();
        b.set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        b.set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        b.append_extension(BasicConstraints::new().build().unwrap())
            .unwrap();
        b.append_extension(
            KeyUsage::new()
                .digital_signature()
                .key_encipherment()
                .build()
                .unwrap(),
        )
        .unwrap();
        if server {
            let san = SubjectAlternativeName::new()
                .dns("localhost")
                .ip("127.0.0.1")
                .build(&b.x509v3_context(Some(&ca.cert), None))
                .unwrap();
            b.append_extension(san).unwrap();
        }
        b.sign(&ca.key, MessageDigest::sha256()).unwrap();
        (b.build(), key)
    }

    fn write_pem(dir: &Path, file: &str, bytes: Vec<u8>) -> String {
        let path = dir.join(file);
        std::fs::write(&path, bytes).expect("write pem");
        path.to_str().unwrap().to_string()
    }

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "actrail-mtls-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A one-shot TLS server that requires a client certificate signed by
    /// `ca_cert_path`. Returns its `https://localhost:port/...` endpoint and a
    /// channel that yields the decrypted request (or an empty string if the
    /// handshake was rejected).
    fn spawn_mtls_server(
        server_cert: String,
        server_key: String,
        ca_cert_path: String,
    ) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let endpoint = format!("https://localhost:{port}/v1/traces");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut acc = SslAcceptor::mozilla_intermediate(SslMethod::tls_server()).unwrap();
            acc.set_private_key_file(&server_key, SslFiletype::PEM)
                .unwrap();
            acc.set_certificate_file(&server_cert, SslFiletype::PEM)
                .unwrap();
            acc.set_ca_file(&ca_cert_path).unwrap();
            acc.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
            let acc = acc.build();
            let (tcp, _) = listener.accept().unwrap();
            match acc.accept(tcp) {
                Ok(mut ssl) => {
                    let mut buf = [0u8; 2048];
                    let n = ssl.read(&mut buf).unwrap_or(0);
                    let _ = ssl.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                    let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
                }
                Err(_) => {
                    // Handshake rejected (e.g. missing/untrusted client cert).
                    let _ = tx.send(String::new());
                }
            }
        });
        (endpoint, rx)
    }

    #[test]
    fn mtls_post_succeeds_and_presents_client_cert() {
        let dir = tmpdir();
        let ca = mk_ca();
        let (srv_cert, srv_key) = mk_leaf(&ca, "collector", true);
        let (cli_cert, cli_key) = mk_leaf(&ca, "actraild", false);
        let ca_path = write_pem(&dir, "ca.pem", ca.cert.to_pem().unwrap());
        let srv_cert_p = write_pem(&dir, "srv.crt", srv_cert.to_pem().unwrap());
        let srv_key_p = write_pem(&dir, "srv.key", srv_key.private_key_to_pem_pkcs8().unwrap());
        let cli_cert_p = write_pem(&dir, "cli.crt", cli_cert.to_pem().unwrap());
        let cli_key_p = write_pem(&dir, "cli.key", cli_key.private_key_to_pem_pkcs8().unwrap());

        let (endpoint, rx) = spawn_mtls_server(srv_cert_p, srv_key_p, ca_path.clone());
        let tls = OtelHttpTlsConfig {
            ca_cert_path: Some(ca_path),
            client_cert_path: Some(cli_cert_p),
            client_key_path: Some(cli_key_p),
        };
        let result = post_json(
            &Endpoint::parse(&endpoint).unwrap(),
            b"{\"resourceSpans\":[]}",
            "application/json",
            None,
            Duration::from_secs(2),
            Duration::from_secs(2),
            &tls,
        );
        assert!(result.is_ok(), "mTLS POST should succeed: {result:?}");
        let request = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("server got request");
        assert!(
            request.starts_with("POST /v1/traces HTTP/1.1"),
            "server received the decrypted HTTP request over TLS: {request:?}"
        );
    }

    #[test]
    fn mtls_server_rejects_client_without_certificate() {
        let dir = tmpdir();
        let ca = mk_ca();
        let (srv_cert, srv_key) = mk_leaf(&ca, "collector", true);
        let ca_path = write_pem(&dir, "ca.pem", ca.cert.to_pem().unwrap());
        let srv_cert_p = write_pem(&dir, "srv.crt", srv_cert.to_pem().unwrap());
        let srv_key_p = write_pem(&dir, "srv.key", srv_key.private_key_to_pem_pkcs8().unwrap());

        let (endpoint, _rx) = spawn_mtls_server(srv_cert_p, srv_key_p, ca_path.clone());
        // Trust the server, but present NO client identity.
        let tls = OtelHttpTlsConfig {
            ca_cert_path: Some(ca_path),
            client_cert_path: None,
            client_key_path: None,
        };
        let result = post_json(
            &Endpoint::parse(&endpoint).unwrap(),
            b"{\"resourceSpans\":[]}",
            "application/json",
            None,
            Duration::from_secs(2),
            Duration::from_secs(2),
            &tls,
        );
        assert!(
            result.is_err(),
            "server requiring a client cert must reject an anonymous client"
        );
    }

    #[test]
    fn client_rejects_server_signed_by_untrusted_ca() {
        let dir = tmpdir();
        let server_ca = mk_ca();
        let other_ca = mk_ca();
        let (srv_cert, srv_key) = mk_leaf(&server_ca, "collector", true);
        let (cli_cert, cli_key) = mk_leaf(&server_ca, "actraild", false);
        let server_ca_p = write_pem(&dir, "sca.pem", server_ca.cert.to_pem().unwrap());
        let other_ca_p = write_pem(&dir, "oca.pem", other_ca.cert.to_pem().unwrap());
        let srv_cert_p = write_pem(&dir, "srv.crt", srv_cert.to_pem().unwrap());
        let srv_key_p = write_pem(&dir, "srv.key", srv_key.private_key_to_pem_pkcs8().unwrap());
        let cli_cert_p = write_pem(&dir, "cli.crt", cli_cert.to_pem().unwrap());
        let cli_key_p = write_pem(&dir, "cli.key", cli_key.private_key_to_pem_pkcs8().unwrap());

        let (endpoint, _rx) = spawn_mtls_server(srv_cert_p, srv_key_p, server_ca_p);
        // Client trusts the WRONG CA -> server cert must not verify.
        let tls = OtelHttpTlsConfig {
            ca_cert_path: Some(other_ca_p),
            client_cert_path: Some(cli_cert_p),
            client_key_path: Some(cli_key_p),
        };
        let result = post_json(
            &Endpoint::parse(&endpoint).unwrap(),
            b"{\"resourceSpans\":[]}",
            "application/json",
            None,
            Duration::from_secs(2),
            Duration::from_secs(2),
            &tls,
        );
        assert!(
            result.is_err(),
            "a server cert signed by an untrusted CA must fail verification"
        );
    }
}
