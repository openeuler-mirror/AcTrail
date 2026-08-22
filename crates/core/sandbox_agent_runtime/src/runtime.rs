use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use sandbox_observation::Observation;

use crate::sender::{ObservationSender, SenderMetrics};
use crate::{
    GuestResourceSource, ProcessIoSource, SandboxAgentConfig, SandboxAgentSnapshot,
    SandboxTransport,
};

pub struct SandboxAgent {
    stop: Arc<AtomicBool>,
    metrics: Arc<AgentMetrics>,
    sender_metrics: Arc<SenderMetrics>,
    workers: WorkerSet,
}

struct WorkerSet {
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl WorkerSet {
    fn new(stop: Arc<AtomicBool>) -> Self {
        Self {
            stop,
            handles: Vec::with_capacity(3),
        }
    }

    fn push(&mut self, handle: JoinHandle<()>) {
        self.handles.push(handle);
    }

    fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        for handle in &self.handles {
            handle.thread().unpark();
        }
        let mut panicked = false;
        for handle in self.handles.drain(..) {
            panicked |= handle.join().is_err();
        }
        if panicked {
            Err(io::Error::other("sandbox agent worker panicked"))
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

struct AgentMetrics {
    io_observations: AtomicU64,
    resource_observations: AtomicU64,
    source_failures: AtomicU64,
    dropped_observations: AtomicU64,
}

impl AgentMetrics {
    fn new() -> Self {
        Self {
            io_observations: AtomicU64::new(0),
            resource_observations: AtomicU64::new(0),
            source_failures: AtomicU64::new(0),
            dropped_observations: AtomicU64::new(0),
        }
    }
}

impl SandboxAgent {
    pub fn start(
        config: SandboxAgentConfig,
        mut io_source: Box<dyn ProcessIoSource>,
        mut resource_source: Box<dyn GuestResourceSource>,
        transport: Arc<dyn SandboxTransport>,
    ) -> io::Result<Self> {
        config.validate()?;
        let (initial_connection, sb_id) = ObservationSender::register(&*transport)?;
        let (sender, receiver) = mpsc::sync_channel(config.observation_queue_capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let metrics = Arc::new(AgentMetrics::new());
        let sender_metrics = Arc::new(SenderMetrics::new(sb_id));
        let mut workers = WorkerSet::new(Arc::clone(&stop));

        workers.push(spawn_source_thread(
            "actrail-sb-io",
            config.worker_thread_stack_bytes,
            Arc::clone(&stop),
            Arc::clone(&metrics),
            sender.clone(),
            config.io_poll_interval,
            move || {
                io_source
                    .poll()
                    .map(|values| values.into_iter().map(Observation::ProcessIo).collect())
            },
            true,
        )?);
        workers.push(spawn_source_thread(
            "actrail-sb-resource",
            config.worker_thread_stack_bytes,
            Arc::clone(&stop),
            Arc::clone(&metrics),
            sender,
            config.resource_poll_interval,
            move || {
                resource_source
                    .sample()
                    .map(|value| vec![Observation::GuestResource(value)])
            },
            false,
        )?);
        let thread_stop = Arc::clone(&stop);
        let thread_sender_metrics = Arc::clone(&sender_metrics);
        let sender_worker = ObservationSender::new(
            config.clone(),
            transport,
            receiver,
            thread_stop,
            thread_sender_metrics,
        );
        workers.push(
            thread::Builder::new()
                .name("actrail-sb-vsock".to_string())
                .stack_size(config.worker_thread_stack_bytes)
                .spawn(move || sender_worker.run(initial_connection))?,
        );
        Ok(Self {
            stop,
            metrics,
            sender_metrics,
            workers,
        })
    }

    pub fn snapshot(&self) -> SandboxAgentSnapshot {
        SandboxAgentSnapshot {
            sb_id: self.sender_metrics.sb_id.load(Ordering::Acquire),
            collected_io_observations: self.metrics.io_observations.load(Ordering::Relaxed),
            collected_resource_observations: self
                .metrics
                .resource_observations
                .load(Ordering::Relaxed),
            source_failures: self.metrics.source_failures.load(Ordering::Relaxed),
            dropped_observations: self.metrics.dropped_observations.load(Ordering::Relaxed),
            sent_batches: self.sender_metrics.sent_batches.load(Ordering::Relaxed),
            reconnects: self.sender_metrics.reconnects.load(Ordering::Relaxed),
        }
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        self.workers.shutdown()
    }
}

impl Drop for SandboxAgent {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn spawn_source_thread<F>(
    name: &str,
    stack_size: usize,
    stop: Arc<AtomicBool>,
    metrics: Arc<AgentMetrics>,
    sender: SyncSender<Observation>,
    interval: std::time::Duration,
    mut poll: F,
    io_source: bool,
) -> io::Result<JoinHandle<()>>
where
    F: FnMut() -> io::Result<Vec<Observation>> + Send + 'static,
{
    thread::Builder::new()
        .name(name.to_string())
        .stack_size(stack_size)
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match poll() {
                    Ok(values) => {
                        for value in values {
                            match sender.try_send(value) {
                                Ok(()) => {
                                    if io_source {
                                        metrics.io_observations.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        metrics
                                            .resource_observations
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                Err(TrySendError::Full(_)) => {
                                    metrics.dropped_observations.fetch_add(1, Ordering::Relaxed);
                                }
                                Err(TrySendError::Disconnected(_)) => return,
                            }
                        }
                    }
                    Err(_) => {
                        metrics.source_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
                thread::park_timeout(interval);
            }
        })
}
