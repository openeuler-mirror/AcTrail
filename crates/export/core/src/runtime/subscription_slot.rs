use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use super::subscription_worker::{
    ObservationWorkItem, ObservationWorkerControl, QueuedObservationBatch, run_observation_worker,
};
use super::{
    ExportDroppedRecord, ExportPublishReport, PostTraceCompletion, SemanticActionExportBatch,
};
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use plugin_system::{
    ObservationBatch, ObservationConsumeReport, ObservationConsumer, ObservationEventFamily,
    PluginHostcallMetricsSource, PluginInstanceStatus, PluginLifecycleState, PluginPurpose,
    PluginRuntimeKind, PostTraceTask,
};

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
    has_post_trace_analyzer: bool,
}

impl ObservationConsumerSlot {
    pub(super) fn new(
        consumer: Box<dyn ObservationConsumer>,
        warnings: Vec<String>,
        post_trace_completion_sender: Sender<PostTraceCompletion>,
    ) -> Self {
        let instance_id = consumer.instance_id().to_string();
        let plugin_id = consumer.plugin_id().to_string();
        let runtime = consumer.runtime_kind();
        let host_grants = consumer.host_grants();
        let hostcall_metrics = consumer.hostcall_metrics_source();
        let event_families = consumer.subscribed_event_families();
        let payload_snapshot_limit = consumer.payload_snapshot_limit();
        let has_post_trace_analyzer = consumer.post_trace_analyzer().is_some();
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
                let consumer = Arc::<dyn ObservationConsumer>::from(consumer);
                let (sender, receiver) = sync_channel(queue_capacity as usize);
                let worker_metrics = Arc::clone(&metrics);
                let worker_consumer = Arc::clone(&consumer);
                let control = Arc::new(ObservationWorkerControl::default());
                let worker_control = Arc::clone(&control);
                let worker_instance_id = instance_id.clone();
                let worker = thread::spawn(move || {
                    run_observation_worker(
                        worker_consumer,
                        receiver,
                        worker_metrics,
                        worker_control,
                        worker_instance_id,
                        Some(queue_capacity),
                        post_trace_completion_sender,
                    );
                });
                (
                    Some(queue_capacity),
                    ObservationDelivery::Queued {
                        sender: Some(sender),
                        worker: Some(worker),
                        consumer,
                        control,
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
            has_post_trace_analyzer,
        }
    }

    pub(super) fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub(super) fn payload_snapshot_limit(&self) -> Option<usize> {
        self.payload_snapshot_limit
    }

    pub(super) fn has_post_trace_analyzer(&self) -> bool {
        self.has_post_trace_analyzer
    }

    pub(super) fn enqueue_post_trace(&self, task: PostTraceTask) -> Result<(), crate::ExportError> {
        if !self.has_post_trace_analyzer {
            return Err(crate::ExportError::new(
                "post_trace_plugin_contract",
                format!(
                    "plugin instance {} does not export a post-trace analyzer",
                    self.instance_id
                ),
            ));
        }
        let ObservationDelivery::Queued {
            sender: Some(sender),
            ..
        } = &self.delivery
        else {
            return Err(crate::ExportError::new(
                "post_trace_plugin_contract",
                format!(
                    "plugin instance {} has no asynchronous worker",
                    self.instance_id
                ),
            ));
        };
        if !self.try_reserve_queue_slot() {
            return Err(crate::ExportError::new(
                "post_trace_queue_full",
                format!("plugin instance {} queue is full", self.instance_id),
            ));
        }
        match sender.try_send(ObservationWorkItem::PostTrace(task)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.release_queue_slot();
                Err(crate::ExportError::new(
                    "post_trace_queue_full",
                    format!("plugin instance {} queue is full", self.instance_id),
                ))
            }
            Err(TrySendError::Disconnected(_)) => {
                self.release_queue_slot();
                Err(crate::ExportError::new(
                    "post_trace_queue_stopped",
                    format!("plugin instance {} queue is stopped", self.instance_id),
                ))
            }
        }
    }

    pub(super) fn cancel_post_trace(&self) -> Result<(), crate::ExportError> {
        let ObservationDelivery::Queued {
            consumer, control, ..
        } = &self.delivery
        else {
            return Err(crate::ExportError::new(
                "post_trace_plugin_contract",
                format!(
                    "plugin instance {} has no asynchronous worker",
                    self.instance_id
                ),
            ));
        };
        let analyzer = consumer.post_trace_analyzer().ok_or_else(|| {
            crate::ExportError::new(
                "post_trace_plugin_contract",
                format!(
                    "plugin instance {} does not export a post-trace analyzer",
                    self.instance_id
                ),
            )
        })?;
        control.request_cancellation();
        analyzer.cancel_post_trace();
        Ok(())
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
                    file_observation_paths: batch.file_observation_paths,
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
        if let ObservationDelivery::Queued {
            sender,
            worker,
            consumer,
            control,
        } = &mut self.delivery
        {
            if let Some(analyzer) = consumer.post_trace_analyzer() {
                control.request_cancellation();
                analyzer.cancel_post_trace();
            }
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
        sender: Option<SyncSender<ObservationWorkItem>>,
        worker: Option<JoinHandle<()>>,
        consumer: Arc<dyn ObservationConsumer>,
        control: Arc<ObservationWorkerControl>,
    },
}

pub(super) struct ObservationConsumerMetrics {
    pub(super) observed_records: AtomicU64,
    pub(super) dropped_records: AtomicU64,
    pub(super) queue_depth: AtomicU64,
    pub(super) last_error: Mutex<Option<String>>,
    pub(super) pending_dropped_records: Mutex<Vec<ExportDroppedRecord>>,
}

fn enqueue_observation_batch(
    slot: &ObservationConsumerSlot,
    sender: &SyncSender<ObservationWorkItem>,
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
        file_observation_paths: batch.file_observation_paths.to_vec(),
        payload_segments: payload_segments.to_vec(),
    };
    match sender.try_send(ObservationWorkItem::Batch(queued_batch)) {
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

pub(super) fn record_successful_consume(
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

pub(super) fn record_consume_error(
    metrics: &ObservationConsumerMetrics,
    dropped_records: u64,
    reason: String,
) {
    metrics
        .dropped_records
        .fetch_add(dropped_records, Ordering::Relaxed);
    store_last_error(metrics, Some(reason));
}

pub(super) fn record_pending_drop(metrics: &ObservationConsumerMetrics, drop: ExportDroppedRecord) {
    if drop.dropped_records == u64::default() {
        return;
    }
    if let Ok(mut pending) = metrics.pending_dropped_records.lock() {
        pending.push(drop);
    }
}

pub(super) fn store_last_error(metrics: &ObservationConsumerMetrics, error: Option<String>) {
    if let Ok(mut last_error) = metrics.last_error.lock() {
        *last_error = error;
    }
}
