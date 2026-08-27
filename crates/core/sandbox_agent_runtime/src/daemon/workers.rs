use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use sandbox_observation::Observation;

use crate::delivery::{DeliveryCounts, DeliveryOutcome, DeliveryPipeline};
use crate::status::DaemonMetrics;
use crate::{GuestResourceSource, ProcessIoSource};

pub(crate) struct BaselineRequest {
    pub(crate) publication_generation: Option<u64>,
    pub(crate) response: SyncSender<io::Result<()>>,
}

pub(crate) struct WorkerSet {
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl WorkerSet {
    pub(super) fn new(stop: Arc<AtomicBool>) -> Self {
        Self {
            stop,
            handles: Vec::with_capacity(3),
        }
    }

    pub(super) fn push(&mut self, handle: JoinHandle<()>) {
        self.handles.push(handle);
    }

    pub(super) fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        for handle in &self.handles {
            handle.thread().unpark();
        }
        let mut panicked = false;
        for handle in self.handles.drain(..) {
            panicked |= handle.join().is_err();
        }
        if panicked {
            Err(io::Error::other("sandbox daemon worker panicked"))
        } else {
            Ok(())
        }
    }
}

impl Drop for WorkerSet {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

pub(crate) fn spawn_io_worker(
    stack_size: usize,
    stop: Arc<AtomicBool>,
    metrics: Arc<DaemonMetrics>,
    delivery: DeliveryPipeline,
    interval: Duration,
    baseline_commands: Receiver<BaselineRequest>,
    mut source: Box<dyn ProcessIoSource>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("actrail-sb-io".to_string())
        .stack_size(stack_size)
        .spawn(move || {
            thread::park_timeout(interval);
            while !stop.load(Ordering::Acquire) {
                match baseline_commands.try_recv() {
                    Ok(request) => {
                        let result = match request.publication_generation {
                            Some(generation) => source.activate_publication(generation),
                            None => source.establish_baseline(),
                        };
                        let _ = request.response.send(result);
                        thread::park_timeout(interval);
                        continue;
                    }
                    Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => {}
                }
                let generation = delivery.capture_generation();
                match source.poll() {
                    Ok(observations) => {
                        let count = observations.len();
                        let counts = match generation {
                            Some(generation) if delivery.generation_is_current(generation) => {
                                delivery.publish_iter(generation, count, observations)
                            }
                            _ => DeliveryCounts::all_dropped(count),
                        };
                        metrics.record_observations(true, counts.accepted, counts.dropped);
                    }
                    Err(_) => metrics.record_source_failure(),
                }
                thread::park_timeout(interval);
            }
        })
}

pub(crate) fn spawn_resource_worker(
    stack_size: usize,
    stop: Arc<AtomicBool>,
    metrics: Arc<DaemonMetrics>,
    delivery: DeliveryPipeline,
    interval: Duration,
    mut source: Box<dyn GuestResourceSource>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("actrail-sb-resource".to_string())
        .stack_size(stack_size)
        .spawn(move || {
            thread::park_timeout(interval);
            while !stop.load(Ordering::Acquire) {
                let generation = delivery.capture_generation();
                match source.sample() {
                    Ok(snapshot) => {
                        let outcome = match generation {
                            Some(generation) => delivery
                                .publish_for(generation, Observation::GuestResource(snapshot)),
                            None => DeliveryOutcome::Dropped,
                        };
                        let (accepted, dropped) = match outcome {
                            DeliveryOutcome::Accepted => (1, 0),
                            DeliveryOutcome::Dropped => (0, 1),
                        };
                        metrics.record_observations(false, accepted, dropped);
                    }
                    Err(_) => metrics.record_source_failure(),
                }
                thread::park_timeout(interval);
            }
        })
}
