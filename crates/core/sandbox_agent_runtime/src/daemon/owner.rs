use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, sync_channel};
use std::thread;

use sandbox_control::{SandboxControlPort, SandboxControlStatus};

use super::{SandboxAgentControlHandle, WorkerSet, spawn_io_worker, spawn_resource_worker};
use crate::delivery::{ConnectionGate, DeliveryPipeline, DeliveryQueue};
use crate::session::{SessionOwner, SessionWake, SharedSessionStatus};
use crate::status::DaemonMetrics;
use crate::{
    GuestResourceSource, ProcessIoSource, SandboxAgentConfig, SandboxAgentSnapshot,
    SandboxTransportFactory,
};

pub struct SandboxAgentDaemon {
    stop: Arc<AtomicBool>,
    gate: Arc<ConnectionGate>,
    metrics: Arc<DaemonMetrics>,
    status: Arc<SharedSessionStatus>,
    control: SandboxAgentControlHandle,
    workers: WorkerSet,
}

impl SandboxAgentDaemon {
    pub fn start(
        config: SandboxAgentConfig,
        mut io_source: Box<dyn ProcessIoSource>,
        mut resource_source: Box<dyn GuestResourceSource>,
        transport: Arc<dyn SandboxTransportFactory>,
    ) -> io::Result<Self> {
        config.validate()?;

        // Startup is fail-fast and establishes template-safe baselines before ready.
        io_source.establish_baseline()?;
        resource_source.sample()?;

        let gate = Arc::new(ConnectionGate::disconnected());
        let metrics = Arc::new(DaemonMetrics::new(config.metrics_enabled));
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(SharedSessionStatus::ready());
        let wake = Arc::new(SessionWake::new());
        let (observation_sender, observation_receiver) =
            mpsc::sync_channel(config.observation_queue_capacity);
        let delivery =
            DeliveryPipeline::new(Arc::clone(&gate), observation_sender, Arc::clone(&wake));
        let (baseline_sender, baseline_receiver) = sync_channel(1);
        let (command_sender, command_receiver) = sync_channel(1);
        let mut workers = WorkerSet::new(Arc::clone(&stop));

        let io_worker = spawn_io_worker(
            config.worker_thread_stack_bytes,
            Arc::clone(&stop),
            Arc::clone(&metrics),
            delivery.clone(),
            config.io_poll_interval,
            baseline_receiver,
            io_source,
        )?;
        let io_thread = io_worker.thread().clone();
        workers.push(io_worker);

        workers.push(spawn_resource_worker(
            config.worker_thread_stack_bytes,
            Arc::clone(&stop),
            Arc::clone(&metrics),
            delivery,
            config.resource_poll_interval,
            resource_source,
        )?);

        let session = SessionOwner::new(
            config.clone(),
            transport,
            Arc::clone(&gate),
            DeliveryQueue::new(observation_receiver),
            command_receiver,
            baseline_sender,
            io_thread,
            Arc::clone(&status),
            Arc::clone(&metrics),
            Arc::clone(&stop),
            Arc::clone(&wake),
            Vec::with_capacity(config.batch_max_observations),
        );
        workers.push(
            thread::Builder::new()
                .name("actrail-sb-vsock".to_string())
                .stack_size(config.worker_thread_stack_bytes)
                .spawn(move || session.run())?,
        );

        Ok(Self {
            stop,
            gate,
            metrics,
            status: Arc::clone(&status),
            control: SandboxAgentControlHandle::new(
                command_sender,
                status,
                wake,
                config.control_request_timeout,
            ),
            workers,
        })
    }

    pub fn control_port(&self) -> SandboxAgentControlHandle {
        self.control.clone()
    }

    pub fn status(&self) -> SandboxControlStatus {
        self.control.status()
    }

    pub fn snapshot(&self) -> SandboxAgentSnapshot {
        self.metrics.snapshot()
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        self.gate.disable();
        self.status.stopping();
        self.control.shutdown();
        self.workers.shutdown()
    }
}

impl Drop for SandboxAgentDaemon {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
