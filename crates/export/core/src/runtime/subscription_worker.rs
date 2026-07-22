use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use model_core::payload::PayloadSegment;
use model_core::trace::TraceRecord;
use plugin_system::{ObservationBatch, ObservationConsumer, PluginRuntimeError, PostTraceTask};
use semantic_action::{FileObservationPath, SemanticAction, SemanticActionLink};

use super::subscription_slot::{
    ObservationConsumerMetrics, record_consume_error, record_pending_drop,
    record_successful_consume, store_last_error,
};
use super::{ExportDroppedRecord, PostTraceCompletion};

pub(super) struct QueuedObservationBatch {
    pub(super) trace: TraceRecord,
    pub(super) semantic_actions: Vec<SemanticAction>,
    pub(super) semantic_links: Vec<SemanticActionLink>,
    pub(super) file_observation_paths: Vec<FileObservationPath>,
    pub(super) payload_segments: Vec<PayloadSegment>,
}

pub(super) enum ObservationWorkItem {
    Batch(QueuedObservationBatch),
    PostTrace(PostTraceTask),
}

#[derive(Default)]
pub(super) struct ObservationWorkerControl {
    cancellation_requested: AtomicBool,
}

impl ObservationWorkerControl {
    pub(super) fn request_cancellation(&self) {
        self.cancellation_requested.store(true, Ordering::Release);
    }

    fn cancellation_requested(&self) -> bool {
        self.cancellation_requested.load(Ordering::Acquire)
    }
}

pub(super) fn run_observation_worker(
    consumer: Arc<dyn ObservationConsumer>,
    receiver: Receiver<ObservationWorkItem>,
    metrics: Arc<ObservationConsumerMetrics>,
    control: Arc<ObservationWorkerControl>,
    instance_id: String,
    queue_capacity: Option<u32>,
    post_trace_completion_sender: Sender<PostTraceCompletion>,
) {
    while let Ok(work_item) = receiver.recv() {
        match work_item {
            ObservationWorkItem::Batch(batch) => {
                if control.cancellation_requested() {
                    cancel_observation_batch(batch, &metrics, &instance_id, queue_capacity);
                } else {
                    run_observation_batch(
                        consumer.as_ref(),
                        batch,
                        &metrics,
                        &instance_id,
                        queue_capacity,
                    );
                }
            }
            ObservationWorkItem::PostTrace(task) => {
                if control.cancellation_requested() {
                    complete_cancelled_post_trace(
                        task.trace_id,
                        &metrics,
                        &instance_id,
                        &post_trace_completion_sender,
                    );
                } else {
                    run_post_trace_task(
                        consumer.as_ref(),
                        task,
                        &metrics,
                        &control,
                        &instance_id,
                        &post_trace_completion_sender,
                    );
                }
            }
        }
        metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
    }
}

fn cancel_observation_batch(
    batch: QueuedObservationBatch,
    metrics: &ObservationConsumerMetrics,
    instance_id: &str,
    queue_capacity: Option<u32>,
) {
    let dropped_records = u64::try_from(batch.semantic_actions.len()).unwrap_or(u64::MAX);
    let reason = "plugin worker cancelled during lifecycle drain".to_string();
    record_pending_drop(
        metrics,
        ExportDroppedRecord {
            trace_id: batch.trace.trace_id,
            exporter: instance_id.to_string(),
            reason: reason.clone(),
            queue_capacity,
            dropped_records,
        },
    );
    record_consume_error(metrics, dropped_records, reason);
}

fn run_observation_batch(
    consumer: &dyn ObservationConsumer,
    batch: QueuedObservationBatch,
    metrics: &ObservationConsumerMetrics,
    instance_id: &str,
    queue_capacity: Option<u32>,
) {
    let action_count = u64::try_from(batch.semantic_actions.len()).unwrap_or(u64::MAX);
    let trace_id = batch.trace.trace_id;
    let result = catch_unwind(AssertUnwindSafe(|| {
        consumer.consume(ObservationBatch {
            trace: &batch.trace,
            semantic_actions: &batch.semantic_actions,
            semantic_links: &batch.semantic_links,
            file_observation_paths: &batch.file_observation_paths,
            payload_segments: &batch.payload_segments,
        })
    }));
    match result {
        Ok(Ok(report)) => record_successful_consume(metrics, action_count, report, true),
        Ok(Err(error)) => {
            let reason = format!("{}: {}", error.code, error.message);
            record_pending_drop(
                metrics,
                ExportDroppedRecord {
                    trace_id,
                    exporter: instance_id.to_string(),
                    reason: reason.clone(),
                    queue_capacity,
                    dropped_records: action_count,
                },
            );
            record_consume_error(metrics, action_count, reason);
        }
        Err(panic) => {
            let reason = format!("plugin consumer panicked: {}", panic_message(&panic));
            record_pending_drop(
                metrics,
                ExportDroppedRecord {
                    trace_id,
                    exporter: instance_id.to_string(),
                    reason: reason.clone(),
                    queue_capacity,
                    dropped_records: action_count,
                },
            );
            record_consume_error(metrics, action_count, reason);
        }
    }
}

fn run_post_trace_task(
    consumer: &dyn ObservationConsumer,
    task: PostTraceTask,
    metrics: &ObservationConsumerMetrics,
    control: &ObservationWorkerControl,
    instance_id: &str,
    completion_sender: &Sender<PostTraceCompletion>,
) {
    let trace_id = task.trace_id;
    let result = catch_unwind(AssertUnwindSafe(|| {
        consumer
            .post_trace_analyzer()
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "post_trace_plugin_contract",
                    "post-trace analyzer export disappeared after admission",
                )
            })?
            .analyze_post_trace(task)
    }))
    .unwrap_or_else(|panic| {
        Err(PluginRuntimeError::new(
            "post_trace_plugin_panic",
            format!("plugin analyzer panicked: {}", panic_message(&panic)),
        ))
    });
    let result = match result {
        Err(_) if control.cancellation_requested() => Err(post_trace_cancelled()),
        result => result,
    };
    if let Err(error) = &result {
        store_last_error(metrics, Some(format!("{}: {}", error.code, error.message)));
    }
    let _ = completion_sender.send(PostTraceCompletion {
        trace_id,
        instance_id: instance_id.to_string(),
        result,
    });
}

fn complete_cancelled_post_trace(
    trace_id: model_core::ids::TraceId,
    metrics: &ObservationConsumerMetrics,
    instance_id: &str,
    completion_sender: &Sender<PostTraceCompletion>,
) {
    let error = post_trace_cancelled();
    store_last_error(metrics, Some(format!("{}: {}", error.code, error.message)));
    let _ = completion_sender.send(PostTraceCompletion {
        trace_id,
        instance_id: instance_id.to_string(),
        result: Err(error),
    });
}

fn post_trace_cancelled() -> PluginRuntimeError {
    PluginRuntimeError::new(
        "post_trace_cancelled",
        "post-trace analysis was cancelled during plugin unload or daemon shutdown",
    )
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
