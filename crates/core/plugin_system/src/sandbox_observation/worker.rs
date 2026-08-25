use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use sandbox_plugin_delivery::{
    SandboxConsumerBatch, SandboxConsumerId, SandboxObservationConsumer,
};

use super::registry::ConsumerMetrics;

pub(super) struct SandboxConsumerWorker;

impl SandboxConsumerWorker {
    pub(super) fn spawn(
        consumer_id: SandboxConsumerId,
        name: String,
        consumer: Arc<dyn SandboxObservationConsumer>,
        receiver: Receiver<SandboxConsumerBatch>,
        metrics: Arc<ConsumerMetrics>,
    ) -> std::io::Result<JoinHandle<()>> {
        thread::Builder::new()
            .name(format!("sandbox-plugin-{}", consumer_id.get()))
            .spawn(move || Self::run(name, consumer, receiver, metrics))
    }

    fn run(
        name: String,
        consumer: Arc<dyn SandboxObservationConsumer>,
        receiver: Receiver<SandboxConsumerBatch>,
        metrics: Arc<ConsumerMetrics>,
    ) {
        while let Ok(batch) = receiver.recv() {
            metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            let observation_count =
                u64::try_from(batch.observation_indices().len()).unwrap_or(u64::MAX);
            match catch_unwind(AssertUnwindSafe(|| consumer.consume(batch))) {
                Ok(Ok(report)) => {
                    metrics
                        .observed_records
                        .fetch_add(report.observed_records, Ordering::Relaxed);
                    metrics
                        .dropped_records
                        .fetch_add(report.dropped_records, Ordering::Relaxed);
                }
                Ok(Err(error)) => {
                    metrics
                        .dropped_records
                        .fetch_add(observation_count, Ordering::Relaxed);
                    set_last_error(&metrics, format!("{}: {}", error.code, error.message));
                }
                Err(_) => {
                    metrics
                        .dropped_records
                        .fetch_add(observation_count, Ordering::Relaxed);
                    set_last_error(&metrics, format!("sandbox consumer {name} panicked"));
                    Self::discard_pending(&receiver, &metrics);
                    break;
                }
            }
        }
        metrics.closed.store(true, Ordering::Relaxed);
    }

    fn discard_pending(receiver: &Receiver<SandboxConsumerBatch>, metrics: &ConsumerMetrics) {
        while let Ok(batch) = receiver.try_recv() {
            metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            let dropped = u64::try_from(batch.observation_indices().len()).unwrap_or(u64::MAX);
            metrics
                .dropped_records
                .fetch_add(dropped, Ordering::Relaxed);
        }
    }
}

fn set_last_error(metrics: &ConsumerMetrics, message: String) {
    if let Ok(mut error) = metrics.last_error.lock() {
        *error = Some(message);
    }
}
