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
use semantic_action::attr_keys::{llm_request, process_parent};

use crate::config::{
    Endpoint, OtelAttributeMode, OtelCompression, OtelEncoding, OtelHttpExporterConfig,
    OtelHttpTlsConfig,
};

const OTEL_HTTP_EXPORTER_NAME: &str = "otel_live_http";
const OTEL_HTTP_PLUGIN_ID: &str = "otel-http";
pub const OTEL_HTTP_BUILTIN_PLUGIN_INSTANCE_ID: &str = "builtin.otel-http";
const SENDER_THREAD_NAME: &str = "actrail-live-otel-http";

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
            process_parent::IDENTITY_STATE
                | llm_request::TRAJECTORY_ID
                | llm_request::TRAJECTORY_INFERENCE_VERSION
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
    /// Concatenated `ExportTraceServiceRequest` bytes. `record_ends` holds the
    /// end offset of each appended record, which is both the span count and the
    /// only way to cut the concatenation back apart on a 413.
    Proto {
        bytes: Vec<u8>,
        record_ends: Vec<usize>,
    },
}

impl PendingBatch {
    fn new(encoding: OtelEncoding) -> Self {
        match encoding {
            OtelEncoding::Json => Self::Json(Vec::new()),
            OtelEncoding::Protobuf => Self::Proto {
                bytes: Vec::new(),
                record_ends: Vec::new(),
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
            Self::Proto { record_ends, .. } => record_ends.len(),
        }
    }

    /// Move the second half of the batch into a new batch, leaving the first
    /// half here. `None` when a single record cannot be divided any further.
    fn split_off_half(&mut self) -> Option<Self> {
        if self.len() < 2 {
            return None;
        }
        match self {
            Self::Json(spans) => Some(Self::Json(spans.split_off(spans.len() / 2))),
            Self::Proto { bytes, record_ends } => {
                let middle = record_ends.len() / 2;
                let cut = record_ends[middle - 1];
                let tail_bytes = bytes.split_off(cut);
                let tail_ends = record_ends
                    .split_off(middle)
                    .into_iter()
                    .map(|end| end - cut)
                    .collect();
                Some(Self::Proto {
                    bytes: tail_bytes,
                    record_ends: tail_ends,
                })
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
            (Self::Proto { bytes, record_ends }, EncodedRecord::Proto(record_bytes)) => {
                bytes.extend_from_slice(&record_bytes);
                record_ends.push(bytes.len());
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
    /// Configured extra headers, pre-rendered once as `name: value\r\n` lines
    /// and spliced verbatim into every request head. Empty when unconfigured.
    extra_headers: String,
    connection: Option<HttpConnection>,
    dropped_batches: u64,
    metrics: Arc<OtelHttpOperationalMetrics>,
}

impl HttpBatchSink {
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

/// Classification of a failed POST, per the OTLP/HTTP spec.
#[derive(Debug)]
enum PostError {
    /// Transport error or a retryable status (408/429/500/502/503/504).
    /// `retry_after` carries a server-supplied delay when present.
    Retryable {
        detail: String,
        retry_after: Option<Duration>,
    },
    /// The collector rejected the body as too large (413). Replaying it is
    /// pointless, but the same spans fit in smaller batches.
    TooLarge { detail: String },
    /// A status that replay cannot fix (400, 401, 403, 404, 415, ...). Drop now.
    Permanent { detail: String },
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

#[derive(Debug)]
struct PostSuccess {
    partial_rejected: u64,
}

/// The collector's body-size limit is below this batch's size.
const HTTP_PAYLOAD_TOO_LARGE: u16 = 413;

/// Retryable status codes. Everything else non-2xx is permanent.
///
/// 429/502/503/504 are the OTLP specification's retryable set. 408 and 500 are
/// added on top: both describe the collector failing to process a request it
/// otherwise accepted, so a replay is the only thing that can recover them, and
/// `retry_max_attempts` bounds the cost when a collector returns them forever.
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
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

/// Render the configured headers into the `name: value\r\n` block the request
/// head splices in. Names and values were validated at config parse time, so
/// this cannot introduce a header the transport does not expect.
fn render_extra_headers(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect()
}

struct PostRequest<'a> {
    endpoint: &'a Endpoint,
    body: &'a [u8],
    content_type: &'a str,
    content_encoding: Option<&'a str>,
    extra_headers: &'a str,
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
        request.extra_headers,
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
    if status == HTTP_PAYLOAD_TOO_LARGE {
        Err(PostError::TooLarge { detail })
    } else if is_retryable_status(status) {
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
    extra_headers: &str,
) -> Result<HttpResponse, String> {
    let content_encoding_header = content_encoding
        .map(|encoding| format!("Content-Encoding: {encoding}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: {}\r\n\
         {}{}Content-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        endpoint.path,
        endpoint.authority(),
        content_type,
        content_encoding_header,
        extra_headers,
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
mod request_body_export_tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::{Duration, UNIX_EPOCH};

    use export_core::SemanticActionKindSelection;
    use model_core::ids::{OtelTraceId, ProfileName, TraceId, TraceName};
    use model_core::process::ProcessIdentity;
    use model_core::trace::{TraceAlertToken, TraceRecord};
    use plugin_system::{ObservationBatch, ObservationConsumer};
    use semantic_action::{
        SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
        attr_keys::llm_request,
    };

    use super::build_otel_http_observation_consumer;
    use crate::config::{
        OtelAttributeMode, OtelCompression, OtelEncoding, OtelHttpExporterConfig, OtelHttpTlsConfig,
    };

    fn test_config(endpoint: String, attribute_mode: OtelAttributeMode) -> OtelHttpExporterConfig {
        OtelHttpExporterConfig {
            endpoint,
            allow_insecure: true,
            queue_capacity: 16,
            batch_max_spans: 1,
            batch_timeout_ms: 60_000,
            connect_timeout_ms: 1_000,
            request_timeout_ms: 1_000,
            retry_max_attempts: 2,
            retry_backoff_ms: 10,
            shutdown_flush_deadline_ms: 500,
            tls: OtelHttpTlsConfig::default(),
            encoding: OtelEncoding::Json,
            compression: OtelCompression::None,
            headers: Vec::new(),
            action_kinds: SemanticActionKindSelection::from_config_entries([
                ("default".to_string(), false),
                ("llm.request".to_string(), true),
            ])
            .expect("LLM request export policy"),
            attribute_mode,
        }
    }

    fn spawn_stub_collector() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub collector");
        let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set collector read timeout");
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
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            let _ = sender.send(String::from_utf8_lossy(&request).to_string());
        });
        (endpoint, receiver)
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

    fn export_canonical_request_body(attribute_mode: OtelAttributeMode) -> String {
        let (endpoint, received) = spawn_stub_collector();
        let consumer = build_otel_http_observation_consumer(test_config(endpoint, attribute_mode))
            .expect("build consumer");
        let trace = TraceRecord::new(
            TraceId::new(7),
            OtelTraceId::from_bytes([7; OtelTraceId::BYTE_COUNT]).expect("non-zero OTEL trace ID"),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(100),
            TraceName::new("otel-http-request-body"),
            ProfileName::new("test"),
            UNIX_EPOCH,
        );
        let canonical_body = r#"{"messages":[{"content":"body-export-marker","role":"user"}]}"#;
        let action = SemanticAction {
            action_id: "llm-request-body".to_string(),
            trace_id: TraceId::new(7),
            kind: SemanticActionKind::LlmRequest,
            title: "LLM request".to_string(),
            start_time: UNIX_EPOCH,
            end_time: Some(UNIX_EPOCH + Duration::from_millis(1)),
            process: ProcessIdentity::new(100),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            attributes: BTreeMap::from([(
                llm_request::CANONICAL_BODY_JSON.to_string(),
                canonical_body.to_string(),
            )]),
            evidence: Vec::new(),
        };

        consumer
            .consume(ObservationBatch {
                trace: &trace,
                trace_finalized: false,
                semantic_actions: std::slice::from_ref(&action),
                semantic_links: &[],
                file_observation_paths: &[],
                payload_segments: &[],
            })
            .expect("consume LLM request body");

        received
            .recv_timeout(Duration::from_secs(2))
            .expect("collector receives the LLM request body")
    }

    #[test]
    fn full_attribute_mode_delivers_canonical_request_body_to_collector() {
        let request = export_canonical_request_body(OtelAttributeMode::Full);

        assert!(request.contains(llm_request::CANONICAL_BODY_JSON));
        assert!(request.contains("body-export-marker"));
    }

    #[test]
    fn metadata_only_mode_keeps_canonical_request_body_out_of_collector() {
        let request = export_canonical_request_body(OtelAttributeMode::MetadataOnly);

        assert!(request.contains("llm.request"));
        assert!(!request.contains(llm_request::CANONICAL_BODY_JSON));
        assert!(!request.contains("body-export-marker"));
    }
}
