use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::trace::TraceRecord;
use plugin_system::{
    ObservationBatch, ObservationConsumeReport, ObservationConsumer, ObservationEventFamily,
    PluginHostcallMetricsSource, PluginInstanceStatus, PluginLifecycleState, PluginPurpose,
    PluginRuntimeKind,
};
use semantic_action::{SemanticAction, SemanticActionLink};

use super::{ExportDroppedRecord, ExportPublishReport, SemanticActionExportBatch};

#[derive(Default)]
pub(super) struct DropAccumulator {
    dropped: BTreeMap<RouteDropKey, u64>,
}

impl DropAccumulator {
    pub(super) fn record(
        &mut self,
        trace_id: TraceId,
        route: String,
        reason: String,
        queue_capacity: Option<u32>,
        dropped_records: u64,
    ) {
        let key = RouteDropKey {
            trace_id,
            route,
            reason,
            queue_capacity,
        };
        self.dropped
            .entry(key)
            .and_modify(|count| *count = count.saturating_add(dropped_records))
            .or_insert(dropped_records);
    }

    pub(super) fn into_report(self) -> ExportPublishReport {
        ExportPublishReport::from_dropped_records(
            self.dropped
                .into_iter()
                .map(|(key, dropped_records)| ExportDroppedRecord {
                    trace_id: key.trace_id,
                    exporter: key.route,
                    reason: key.reason,
                    queue_capacity: key.queue_capacity,
                    dropped_records,
                })
                .collect(),
        )
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RouteDropKey {
    trace_id: TraceId,
    route: String,
    reason: String,
    queue_capacity: Option<u32>,
}

pub(super) struct ObservationConsumerSlot {
    instance_id: String,
    plugin_id: String,
    runtime: PluginRuntimeKind,
    host_grants: Vec<String>,
    event_families: Vec<ObservationEventFamily>,
    warnings: Vec<String>,
    hostcall_metrics: Option<Arc<dyn PluginHostcallMetricsSource>>,
    payload_snapshot_limit: Option<usize>,
    queue_capacity: Option<u32>,
    delivery: ObservationDelivery,
    metrics: Arc<ObservationConsumerMetrics>,
}

impl ObservationConsumerSlot {
    pub(super) fn new(consumer: Box<dyn ObservationConsumer>, warnings: Vec<String>) -> Self {
        let instance_id = consumer.instance_id().to_string();
        let plugin_id = consumer.plugin_id().to_string();
        let runtime = consumer.runtime_kind();
        let host_grants = consumer.host_grants();
        let hostcall_metrics = consumer.hostcall_metrics_source();
        let event_families = consumer.subscribed_event_families();
        let payload_snapshot_limit = consumer.payload_snapshot_limit();
        let metrics = Arc::new(ObservationConsumerMetrics {
            observed_records: AtomicU64::new(0),
            dropped_records: AtomicU64::new(0),
            queue_depth: AtomicU64::new(0),
            last_error: Mutex::new(None),
            pending_dropped_records: Mutex::new(Vec::new()),
        });
        let (queue_capacity, delivery) = match runtime {
            PluginRuntimeKind::Builtin => (None, ObservationDelivery::Inline(consumer)),
            PluginRuntimeKind::Wasm | PluginRuntimeKind::NativeDylib => {
                let queue_capacity = consumer.observation_queue_capacity();
                let (sender, receiver) = sync_channel(queue_capacity as usize);
                let worker_metrics = Arc::clone(&metrics);
                let worker_instance_id = instance_id.clone();
                let worker = thread::spawn(move || {
                    run_observation_worker(
                        consumer,
                        receiver,
                        worker_metrics,
                        worker_instance_id,
                        Some(queue_capacity),
                    );
                });
                (
                    Some(queue_capacity),
                    ObservationDelivery::Queued {
                        sender: Some(sender),
                        worker: Some(worker),
                    },
                )
            }
        };
        Self {
            instance_id,
            plugin_id,
            runtime,
            host_grants,
            event_families,
            warnings,
            hostcall_metrics,
            payload_snapshot_limit,
            queue_capacity,
            delivery,
            metrics,
        }
    }

    pub(super) fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub(super) fn payload_snapshot_limit(&self) -> Option<usize> {
        self.payload_snapshot_limit
    }

    pub(super) fn receives_semantic_action_batch(&self) -> bool {
        self.event_families.iter().any(|family| {
            matches!(
                family,
                ObservationEventFamily::SemanticAction | ObservationEventFamily::SemanticActionLink
            )
        })
    }

    pub(super) fn status(&self, state: PluginLifecycleState) -> PluginInstanceStatus {
        PluginInstanceStatus {
            instance_id: self.instance_id.clone(),
            plugin_id: self.plugin_id.clone(),
            purpose: PluginPurpose::ObservationConsumer,
            runtime: self.runtime,
            state,
            host_grants: self.host_grants.clone(),
            queue_depth: self
                .queue_capacity
                .map(|_| self.metrics.queue_depth.load(Ordering::Relaxed)),
            queue_capacity: self.queue_capacity,
            observed_records: self.metrics.observed_records.load(Ordering::Relaxed),
            dropped_records: self.metrics.dropped_records.load(Ordering::Relaxed),
            hostcall_metrics: self
                .hostcall_metrics
                .as_ref()
                .map(|metrics| metrics.snapshot())
                .unwrap_or_default(),
            last_error: self
                .metrics
                .last_error
                .lock()
                .ok()
                .and_then(|error| error.clone()),
            warnings: self.warnings.clone(),
        }
    }

    pub(super) fn publish(
        &self,
        batch: &SemanticActionExportBatch<'_>,
        payload_segments: &[PayloadSegment],
        dropped: &mut DropAccumulator,
    ) {
        match &self.delivery {
            ObservationDelivery::Inline(consumer) => {
                let observation_batch = ObservationBatch {
                    trace: batch.trace,
                    semantic_actions: batch.actions,
                    semantic_links: batch.links,
                    payload_segments,
                };
                consume_observation_batch(
                    self,
                    consumer.as_ref(),
                    observation_batch,
                    batch.actions.len(),
                    dropped,
                );
            }
            ObservationDelivery::Queued {
                sender: Some(sender),
                ..
            } => enqueue_observation_batch(self, sender, batch, payload_segments, dropped),
            ObservationDelivery::Queued { sender: None, .. } => {
                dropped.record(
                    batch.trace.trace_id,
                    self.instance_id.clone(),
                    "plugin queue is stopped".to_string(),
                    self.queue_capacity,
                    u64::try_from(batch.actions.len()).unwrap_or(u64::MAX),
                );
            }
        }
    }

    pub(super) fn drain_pending_drops(&self, dropped: &mut DropAccumulator) {
        let Ok(mut pending) = self.metrics.pending_dropped_records.lock() else {
            return;
        };
        for drop in pending.drain(..) {
            dropped.record(
                drop.trace_id,
                drop.exporter,
                drop.reason,
                drop.queue_capacity,
                drop.dropped_records,
            );
        }
    }

    pub(super) fn stop(&mut self) {
        if let ObservationDelivery::Queued { sender, worker } = &mut self.delivery {
            sender.take();
            if let Some(worker) = worker.take()
                && worker.join().is_err()
            {
                self.store_last_error(Some("plugin queue worker panicked".to_string()));
            }
        }
    }

    fn try_reserve_queue_slot(&self) -> bool {
        let Some(queue_capacity) = self.queue_capacity.map(u64::from) else {
            return true;
        };
        let mut current = self.metrics.queue_depth.load(Ordering::Relaxed);
        loop {
            if current >= queue_capacity {
                return false;
            }
            match self.metrics.queue_depth.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(next) => current = next,
            }
        }
    }

    fn release_queue_slot(&self) {
        self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
    }

    fn store_last_error(&self, error: Option<String>) {
        store_last_error(&self.metrics, error);
    }
}

impl Drop for ObservationConsumerSlot {
    fn drop(&mut self) {
        self.stop();
    }
}

enum ObservationDelivery {
    Inline(Box<dyn ObservationConsumer>),
    Queued {
        sender: Option<SyncSender<QueuedObservationBatch>>,
        worker: Option<JoinHandle<()>>,
    },
}

struct ObservationConsumerMetrics {
    observed_records: AtomicU64,
    dropped_records: AtomicU64,
    queue_depth: AtomicU64,
    last_error: Mutex<Option<String>>,
    pending_dropped_records: Mutex<Vec<ExportDroppedRecord>>,
}

struct QueuedObservationBatch {
    trace: TraceRecord,
    semantic_actions: Vec<SemanticAction>,
    semantic_links: Vec<SemanticActionLink>,
    payload_segments: Vec<PayloadSegment>,
}

fn enqueue_observation_batch(
    slot: &ObservationConsumerSlot,
    sender: &SyncSender<QueuedObservationBatch>,
    batch: &SemanticActionExportBatch<'_>,
    payload_segments: &[PayloadSegment],
    dropped: &mut DropAccumulator,
) {
    if !slot.try_reserve_queue_slot() {
        let dropped_records = u64::try_from(batch.actions.len()).unwrap_or(u64::MAX);
        slot.metrics
            .dropped_records
            .fetch_add(dropped_records, Ordering::Relaxed);
        dropped.record(
            batch.trace.trace_id,
            slot.instance_id.clone(),
            "observation_queue_full".to_string(),
            slot.queue_capacity,
            dropped_records,
        );
        return;
    }
    let queued_batch = QueuedObservationBatch {
        trace: batch.trace.clone(),
        semantic_actions: batch.actions.to_vec(),
        semantic_links: batch.links.to_vec(),
        payload_segments: payload_segments.to_vec(),
    };
    match sender.try_send(queued_batch) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            slot.release_queue_slot();
            let dropped_records = u64::try_from(batch.actions.len()).unwrap_or(u64::MAX);
            slot.metrics
                .dropped_records
                .fetch_add(dropped_records, Ordering::Relaxed);
            dropped.record(
                batch.trace.trace_id,
                slot.instance_id.clone(),
                "observation_queue_full".to_string(),
                slot.queue_capacity,
                dropped_records,
            );
        }
        Err(TrySendError::Disconnected(_)) => {
            slot.release_queue_slot();
            let dropped_records = u64::try_from(batch.actions.len()).unwrap_or(u64::MAX);
            slot.metrics
                .dropped_records
                .fetch_add(dropped_records, Ordering::Relaxed);
            let reason = "plugin queue worker disconnected".to_string();
            slot.store_last_error(Some(reason.clone()));
            dropped.record(
                batch.trace.trace_id,
                slot.instance_id.clone(),
                reason,
                slot.queue_capacity,
                dropped_records,
            );
        }
    }
}

fn consume_observation_batch(
    slot: &ObservationConsumerSlot,
    consumer: &dyn ObservationConsumer,
    batch: ObservationBatch<'_>,
    action_count: usize,
    dropped: &mut DropAccumulator,
) {
    let dropped_records_on_error = u64::try_from(action_count).unwrap_or(u64::MAX);
    let trace_id = batch.trace.trace_id;
    match consumer.consume(batch) {
        Ok(report) => {
            for drop in &report.dropped_records {
                if drop.dropped_records == u64::default() {
                    continue;
                }
                dropped.record(
                    drop.trace_id,
                    drop.plugin_instance.clone(),
                    drop.reason.clone(),
                    drop.queue_capacity,
                    drop.dropped_records,
                );
            }
            record_successful_consume(&slot.metrics, dropped_records_on_error, report, false);
        }
        Err(error) => {
            let reason = format!("{}: {}", error.code, error.message);
            dropped.record(
                trace_id,
                slot.instance_id.clone(),
                reason.clone(),
                None,
                dropped_records_on_error,
            );
            record_consume_error(&slot.metrics, dropped_records_on_error, reason);
        }
    }
}

fn run_observation_worker(
    consumer: Box<dyn ObservationConsumer>,
    receiver: Receiver<QueuedObservationBatch>,
    metrics: Arc<ObservationConsumerMetrics>,
    instance_id: String,
    queue_capacity: Option<u32>,
) {
    while let Ok(batch) = receiver.recv() {
        let action_count = u64::try_from(batch.semantic_actions.len()).unwrap_or(u64::MAX);
        let trace_id = batch.trace.trace_id;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let observation_batch = ObservationBatch {
                trace: &batch.trace,
                semantic_actions: &batch.semantic_actions,
                semantic_links: &batch.semantic_links,
                payload_segments: &batch.payload_segments,
            };
            consumer.consume(observation_batch)
        }));
        match result {
            Ok(Ok(report)) => record_successful_consume(&metrics, action_count, report, true),
            Ok(Err(error)) => {
                let reason = format!("{}: {}", error.code, error.message);
                record_pending_drop(
                    &metrics,
                    ExportDroppedRecord {
                        trace_id,
                        exporter: instance_id.clone(),
                        reason: reason.clone(),
                        queue_capacity,
                        dropped_records: action_count,
                    },
                );
                record_consume_error(&metrics, action_count, reason);
            }
            Err(panic) => {
                let reason = format!("plugin consumer panicked: {}", panic_message(&panic));
                record_pending_drop(
                    &metrics,
                    ExportDroppedRecord {
                        trace_id,
                        exporter: instance_id.clone(),
                        reason: reason.clone(),
                        queue_capacity,
                        dropped_records: action_count,
                    },
                );
                record_panic_error(&metrics, action_count, reason);
            }
        }
        metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
    }
}

fn record_successful_consume(
    metrics: &ObservationConsumerMetrics,
    observed_records: u64,
    report: ObservationConsumeReport,
    queue_pending_drops: bool,
) {
    metrics
        .observed_records
        .fetch_add(observed_records, Ordering::Relaxed);
    store_last_error(metrics, None);
    for drop in report.dropped_records {
        if drop.dropped_records == u64::default() {
            continue;
        }
        if queue_pending_drops {
            record_pending_drop(
                metrics,
                ExportDroppedRecord {
                    trace_id: drop.trace_id,
                    exporter: drop.plugin_instance,
                    reason: drop.reason,
                    queue_capacity: drop.queue_capacity,
                    dropped_records: drop.dropped_records,
                },
            );
        }
        metrics
            .dropped_records
            .fetch_add(drop.dropped_records, Ordering::Relaxed);
    }
}

fn record_consume_error(
    metrics: &ObservationConsumerMetrics,
    dropped_records: u64,
    reason: String,
) {
    metrics
        .dropped_records
        .fetch_add(dropped_records, Ordering::Relaxed);
    store_last_error(metrics, Some(reason));
}

fn record_panic_error(metrics: &ObservationConsumerMetrics, dropped_records: u64, reason: String) {
    metrics
        .dropped_records
        .fetch_add(dropped_records, Ordering::Relaxed);
    store_last_error(metrics, Some(reason));
}

fn record_pending_drop(metrics: &ObservationConsumerMetrics, drop: ExportDroppedRecord) {
    if drop.dropped_records == u64::default() {
        return;
    }
    if let Ok(mut pending) = metrics.pending_dropped_records.lock() {
        pending.push(drop);
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

fn store_last_error(metrics: &ObservationConsumerMetrics, error: Option<String>) {
    if let Ok(mut last_error) = metrics.last_error.lock() {
        *last_error = error;
    }
}
