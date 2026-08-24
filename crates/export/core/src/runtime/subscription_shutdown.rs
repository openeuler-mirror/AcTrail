//! Deadline-aware observation consumer shutdown helpers.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use plugin_system::ObservationConsumer;

use super::ExportRuntimeFailure;
use super::subscription_slot::{
    ObservationConsumerMetrics, finish_observation_consumer, store_last_error,
};

pub(super) fn wait_until_finished(worker: &JoinHandle<()>, deadline: Option<Instant>) -> bool {
    let Some(deadline) = deadline else {
        return true;
    };
    loop {
        if worker.is_finished() {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        thread::sleep(remaining.min(Duration::from_millis(5)));
    }
}

pub(super) fn finish_consumer_before(
    consumer: Arc<dyn ObservationConsumer>,
    metrics: Arc<ObservationConsumerMetrics>,
    deadline: Option<Instant>,
    instance_id: String,
    queue_capacity: Option<u32>,
) -> Vec<ExportRuntimeFailure> {
    let Some(deadline) = deadline else {
        return finish_observation_consumer(consumer.as_ref(), &metrics);
    };
    if deadline <= Instant::now() {
        return vec![stop_timeout_failure(
            &metrics,
            &instance_id,
            queue_capacity,
            "consumer finish",
        )];
    }
    let (result_sender, result_receiver) = sync_channel(1);
    let finish_metrics = Arc::clone(&metrics);
    let spawn_result = thread::Builder::new()
        .name("actrail-export-finish".to_string())
        .spawn(move || {
            let failures = finish_observation_consumer(consumer.as_ref(), &finish_metrics);
            let _ = result_sender.send(failures);
        });
    if let Err(error) = spawn_result {
        return vec![stop_failure(
            &metrics,
            &instance_id,
            queue_capacity,
            format!("consumer finish worker spawn failed: {error}"),
        )];
    }
    receive_finish_result(
        result_receiver,
        deadline,
        &metrics,
        &instance_id,
        queue_capacity,
    )
}

fn receive_finish_result(
    receiver: Receiver<Vec<ExportRuntimeFailure>>,
    deadline: Instant,
    metrics: &ObservationConsumerMetrics,
    instance_id: &str,
    queue_capacity: Option<u32>,
) -> Vec<ExportRuntimeFailure> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match receiver.recv_timeout(remaining) {
        Ok(failures) => failures,
        Err(RecvTimeoutError::Timeout) => vec![stop_timeout_failure(
            metrics,
            instance_id,
            queue_capacity,
            "consumer finish",
        )],
        Err(RecvTimeoutError::Disconnected) => vec![stop_failure(
            metrics,
            instance_id,
            queue_capacity,
            "plugin consumer finish worker disconnected".to_string(),
        )],
    }
}

pub(super) fn stop_timeout_failure(
    metrics: &ObservationConsumerMetrics,
    instance_id: &str,
    queue_capacity: Option<u32>,
    phase: &str,
) -> ExportRuntimeFailure {
    stop_failure(
        metrics,
        instance_id,
        queue_capacity,
        format!("plugin {phase} exceeded shutdown deadline"),
    )
}

fn stop_failure(
    metrics: &ObservationConsumerMetrics,
    instance_id: &str,
    queue_capacity: Option<u32>,
    reason: String,
) -> ExportRuntimeFailure {
    store_last_error(metrics, Some(reason.clone()));
    ExportRuntimeFailure {
        trace_id: None,
        exporter: instance_id.to_string(),
        reason,
        queue_capacity,
        occurrences: 1,
    }
}
