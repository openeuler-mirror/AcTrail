use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime};

use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::trace::{TraceLifecycleState, TraceRecord};
use plugin_system::{ObservationBatch, ObservationConsumer, PluginRuntimeError, PostTraceTask};
use semantic_action::{FileObservationPath, SemanticAction, SemanticActionLink};

use super::subscription_slot::{
    ObservationConsumerMetrics, record_consume_failure, record_pending_runtime_failure,
    record_successful_consume, store_last_error,
};
use super::{ExportRuntimeFailure, PostTraceCompletion};

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
    let mut scheduled = BTreeMap::<TraceId, ScheduledObservation>::new();
    loop {
        while let Some(batch) = take_due_observation(&mut scheduled, SystemTime::now()) {
            update_schedule(
                &mut scheduled,
                run_observation_batch(
                    consumer.as_ref(),
                    batch,
                    &metrics,
                    &instance_id,
                    queue_capacity,
                ),
            );
        }
        let work_item = match next_wait_duration(&scheduled, SystemTime::now()) {
            Some(timeout) => match receiver.recv_timeout(timeout) {
                Ok(work_item) => work_item,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match receiver.recv() {
                Ok(work_item) => work_item,
                Err(_) => break,
            },
        };
        match work_item {
            ObservationWorkItem::Batch(batch) => {
                update_schedule(
                    &mut scheduled,
                    run_observation_batch(
                        consumer.as_ref(),
                        batch,
                        &metrics,
                        &instance_id,
                        queue_capacity,
                    ),
                );
            }
            ObservationWorkItem::PostTrace(task) => {
                scheduled.remove(&task.trace_id);
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

struct ScheduledObservation {
    requested_at: SystemTime,
    trace: TraceRecord,
}

struct ObservationRun {
    trace: TraceRecord,
    reevaluate_at: Option<SystemTime>,
}

fn run_observation_batch(
    consumer: &dyn ObservationConsumer,
    batch: QueuedObservationBatch,
    metrics: &ObservationConsumerMetrics,
    instance_id: &str,
    queue_capacity: Option<u32>,
) -> ObservationRun {
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
    let reevaluate_at = match result {
        Ok(Ok(report)) => {
            let reevaluate_at = report.reevaluate_at;
            record_successful_consume(metrics, action_count, report, true);
            reevaluate_at
        }
        Ok(Err(error)) => {
            let reason = format!("{}: {}", error.code, error.message);
            record_pending_runtime_failure(
                metrics,
                ExportRuntimeFailure {
                    trace_id: Some(trace_id),
                    exporter: instance_id.to_string(),
                    reason: reason.clone(),
                    queue_capacity,
                    occurrences: 1,
                },
            );
            record_consume_failure(metrics, reason);
            None
        }
        Err(panic) => {
            let reason = format!("plugin consumer panicked: {}", panic_message(&panic));
            record_pending_runtime_failure(
                metrics,
                ExportRuntimeFailure {
                    trace_id: Some(trace_id),
                    exporter: instance_id.to_string(),
                    reason: reason.clone(),
                    queue_capacity,
                    occurrences: 1,
                },
            );
            record_consume_failure(metrics, reason);
            None
        }
    };
    ObservationRun {
        trace: batch.trace,
        reevaluate_at,
    }
}

fn update_schedule(
    scheduled: &mut BTreeMap<TraceId, ScheduledObservation>,
    observation: ObservationRun,
) {
    let trace_id = observation.trace.trace_id;
    scheduled.remove(&trace_id);
    if is_terminal(observation.trace.lifecycle_state) {
        return;
    }
    if let Some(requested_at) = observation.reevaluate_at {
        scheduled.insert(
            trace_id,
            ScheduledObservation {
                requested_at,
                trace: observation.trace,
            },
        );
    }
}

fn take_due_observation(
    scheduled: &mut BTreeMap<TraceId, ScheduledObservation>,
    now: SystemTime,
) -> Option<QueuedObservationBatch> {
    let trace_id = scheduled
        .iter()
        .filter(|(_trace_id, observation)| observation.requested_at <= now)
        .min_by_key(|(_trace_id, observation)| observation.requested_at)
        .map(|(trace_id, _observation)| *trace_id)?;
    let observation = scheduled.remove(&trace_id)?;
    Some(QueuedObservationBatch {
        trace: observation.trace,
        semantic_actions: Vec::new(),
        semantic_links: Vec::new(),
        file_observation_paths: Vec::new(),
        payload_segments: Vec::new(),
    })
}

fn next_wait_duration(
    scheduled: &BTreeMap<TraceId, ScheduledObservation>,
    now: SystemTime,
) -> Option<Duration> {
    scheduled
        .values()
        .map(|observation| {
            observation
                .requested_at
                .duration_since(now)
                .unwrap_or(Duration::ZERO)
        })
        .min()
}

fn is_terminal(state: TraceLifecycleState) -> bool {
    matches!(
        state,
        TraceLifecycleState::Completed | TraceLifecycleState::Exited | TraceLifecycleState::Failed
    )
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

pub(super) fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}
