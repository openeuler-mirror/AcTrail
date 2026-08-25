use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sandbox_agent_runtime::SandboxAgentDaemon;
use sandbox_control::{SandboxConnectResponse, SandboxConnectionState, SandboxControlStatus};
use sandbox_linux_collector::KernelCollectionDiagnostics;

pub(crate) struct SbOutput {
    control_socket_path: std::path::PathBuf,
    periodic: Option<PeriodicDiagnostics>,
}

pub(super) struct CollectorDiagnostics {
    failures: AtomicU64,
    pending_io_drops: AtomicU64,
    aggregate_drops: AtomicU64,
    descendant_tracking_drops: AtomicU64,
    oom_event_drops: AtomicU64,
    oom_comm_drops: AtomicU64,
}

struct PeriodicDiagnostics {
    interval: Duration,
    next_output: Instant,
    collector: Arc<CollectorDiagnostics>,
}

#[derive(Clone, Copy)]
struct CollectorSnapshot {
    failures: u64,
    pending_io_drops: u64,
    aggregate_drops: u64,
    descendant_tracking_drops: u64,
    oom_event_drops: u64,
    oom_comm_drops: u64,
}

impl SbOutput {
    pub(super) fn runtime(
        interval: Option<Duration>,
        control_socket_path: &Path,
    ) -> io::Result<(Self, Option<Arc<CollectorDiagnostics>>)> {
        let mut collector = None;
        let periodic = match interval {
            Some(interval) => {
                let next_output = Instant::now().checked_add(interval).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "actrail-sb diagnostics interval exceeds the platform clock range",
                    )
                })?;
                let diagnostics = Arc::new(CollectorDiagnostics::new());
                collector = Some(Arc::clone(&diagnostics));
                Some(PeriodicDiagnostics {
                    interval,
                    next_output,
                    collector: diagnostics,
                })
            }
            None => None,
        };
        Ok((
            Self {
                control_socket_path: control_socket_path.to_path_buf(),
                periodic,
            },
            collector,
        ))
    }

    pub(super) fn report_ready(&self, status: SandboxControlStatus) {
        Self::stderr(format_args!(
            "actrail-sb daemon ready control_socket={} connected={} publication_enabled={}",
            self.control_socket_path.display(),
            status.connection == SandboxConnectionState::Connected,
            status.publication_enabled,
        ));
    }

    pub(super) fn report_if_due(&mut self, agent: &SandboxAgentDaemon) {
        let Some(periodic) = &mut self.periodic else {
            return;
        };
        let now = Instant::now();
        if now < periodic.next_output {
            return;
        }
        let status = agent.status();
        let snapshot = agent.snapshot();
        let collector = periodic.collector.take_snapshot();
        Self::stderr(format_args!(
            "actrail-sb status sb_id={} generation={} publication_enabled={} io_observations={} resource_observations={} source_failures={} dropped_observations={} sent_batches={} reconnects={} reconnect_failures={} collector_failures={} pending_io_drops={} aggregate_drops={} descendant_tracking_drops={} oom_event_drops={} oom_comm_drops={}",
            status.sb_id,
            status.connection_generation,
            status.publication_enabled,
            snapshot.collected_io_observations,
            snapshot.collected_resource_observations,
            snapshot.source_failures,
            snapshot.dropped_observations,
            snapshot.sent_batches,
            snapshot.reconnects,
            snapshot.reconnect_failures,
            collector.failures,
            collector.pending_io_drops,
            collector.aggregate_drops,
            collector.descendant_tracking_drops,
            collector.oom_event_drops,
            collector.oom_comm_drops,
        ));
        periodic.next_output = now.checked_add(periodic.interval).unwrap_or(now);
    }

    pub(super) fn diagnostics_wait(&self) -> Option<Duration> {
        self.periodic.as_ref().map(|periodic| {
            periodic
                .next_output
                .saturating_duration_since(Instant::now())
        })
    }

    pub(super) fn report_control_server_exit(
        &self,
        error: Option<&sandbox_control_uds::SandboxControlUdsError>,
    ) {
        match error {
            Some(error) => Self::stderr(format_args!(
                "actrail-sb control server unavailable error={error}"
            )),
            None => Self::stderr(format_args!(
                "actrail-sb control server unavailable error=server exited unexpectedly"
            )),
        }
    }

    pub(crate) fn startup_error(error: &dyn fmt::Display) {
        Self::stderr(format_args!("actrail-sb: {error}"));
    }

    pub(crate) fn config_written(path: &Path) {
        Self::stdout(format_args!("wrote actrail-sb config {}", path.display()));
    }

    pub(crate) fn connect_succeeded(response: SandboxConnectResponse) {
        Self::stdout(format_args!(
            "actrail-sb connected sb_id={} generation={} reused={}",
            response.sb_id(),
            response.connection_generation(),
            response.reused(),
        ));
    }

    fn stdout(message: fmt::Arguments<'_>) {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        let _ = writeln!(output, "{message}");
    }

    fn stderr(message: fmt::Arguments<'_>) {
        let stderr = io::stderr();
        let mut output = stderr.lock();
        let _ = writeln!(output, "{message}");
    }
}

impl CollectorDiagnostics {
    fn new() -> Self {
        Self {
            failures: AtomicU64::new(0),
            pending_io_drops: AtomicU64::new(0),
            aggregate_drops: AtomicU64::new(0),
            descendant_tracking_drops: AtomicU64::new(0),
            oom_event_drops: AtomicU64::new(0),
            oom_comm_drops: AtomicU64::new(0),
        }
    }

    pub(super) fn record(&self, failures: usize, kernel: KernelCollectionDiagnostics) {
        self.failures.fetch_add(failures as u64, Ordering::Relaxed);
        self.pending_io_drops
            .fetch_add(kernel.pending_io_drops, Ordering::Relaxed);
        self.aggregate_drops
            .fetch_add(kernel.aggregate_drops, Ordering::Relaxed);
        self.descendant_tracking_drops
            .fetch_add(kernel.descendant_tracking_drops, Ordering::Relaxed);
        self.oom_event_drops
            .fetch_add(kernel.oom_event_drops, Ordering::Relaxed);
        self.oom_comm_drops
            .fetch_add(kernel.oom_comm_drops, Ordering::Relaxed);
    }

    fn take_snapshot(&self) -> CollectorSnapshot {
        CollectorSnapshot {
            failures: self.failures.swap(0, Ordering::Relaxed),
            pending_io_drops: self.pending_io_drops.swap(0, Ordering::Relaxed),
            aggregate_drops: self.aggregate_drops.swap(0, Ordering::Relaxed),
            descendant_tracking_drops: self.descendant_tracking_drops.swap(0, Ordering::Relaxed),
            oom_event_drops: self.oom_event_drops.swap(0, Ordering::Relaxed),
            oom_comm_drops: self.oom_comm_drops.swap(0, Ordering::Relaxed),
        }
    }
}
