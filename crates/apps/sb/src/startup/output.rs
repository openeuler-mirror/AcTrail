use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sandbox_agent_runtime::SandboxAgent;
use sandbox_linux_collector::KernelCollectionDiagnostics;

pub struct SbOutput {
    periodic: Option<PeriodicDiagnostics>,
}

pub(super) struct CollectorDiagnostics {
    failures: AtomicU64,
    pending_io_drops: AtomicU64,
    aggregate_drops: AtomicU64,
    descendant_tracking_drops: AtomicU64,
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
}

impl SbOutput {
    pub(super) fn runtime(
        interval: Option<Duration>,
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
        Ok((Self { periodic }, collector))
    }

    pub(super) fn ready(&self, agent: &SandboxAgent) {
        let snapshot = agent.snapshot();
        Self::stderr(format_args!("actrail-sb ready sb_id={}", snapshot.sb_id));
    }

    pub(super) fn report_if_due(&mut self, agent: &SandboxAgent) {
        let Some(periodic) = &mut self.periodic else {
            return;
        };
        let now = Instant::now();
        if now < periodic.next_output {
            return;
        }
        let snapshot = agent.snapshot();
        let collector = periodic.collector.take_snapshot();
        Self::stderr(format_args!(
            "actrail-sb status sb_id={} io_observations={} resource_observations={} source_failures={} dropped_observations={} sent_batches={} reconnects={} collector_failures={} pending_io_drops={} aggregate_drops={} descendant_tracking_drops={}",
            snapshot.sb_id,
            snapshot.collected_io_observations,
            snapshot.collected_resource_observations,
            snapshot.source_failures,
            snapshot.dropped_observations,
            snapshot.sent_batches,
            snapshot.reconnects,
            collector.failures,
            collector.pending_io_drops,
            collector.aggregate_drops,
            collector.descendant_tracking_drops,
        ));
        periodic.next_output = now + periodic.interval;
    }

    pub fn startup_error(error: &dyn fmt::Display) {
        Self::stderr(format_args!("actrail-sb: {error}"));
    }

    pub fn config_written(path: &Path) {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        let _ = writeln!(output, "wrote actrail-sb config {}", path.display());
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
    }

    fn take_snapshot(&self) -> CollectorSnapshot {
        CollectorSnapshot {
            failures: self.failures.swap(0, Ordering::Relaxed),
            pending_io_drops: self.pending_io_drops.swap(0, Ordering::Relaxed),
            aggregate_drops: self.aggregate_drops.swap(0, Ordering::Relaxed),
            descendant_tracking_drops: self.descendant_tracking_drops.swap(0, Ordering::Relaxed),
        }
    }
}
