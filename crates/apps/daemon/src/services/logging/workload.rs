//! Low-overhead daemon workload diagnostics.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use config_core::daemon::WorkloadDiagnosticsConfig;

#[derive(Clone, Default)]
pub(crate) struct WorkloadDiagnostics {
    inner: Option<Arc<WorkloadDiagnosticsInner>>,
}

impl WorkloadDiagnostics {
    pub(crate) fn new(config: WorkloadDiagnosticsConfig) -> Self {
        if !config.enabled {
            return Self { inner: None };
        }
        Self {
            inner: Some(Arc::new(WorkloadDiagnosticsInner {
                interval: Duration::from_millis(config.interval_ms),
                started: AtomicBool::new(false),
                counters: WorkloadCounters::default(),
            })),
        }
    }

    pub(crate) fn start(&self) {
        let Some(inner) = &self.inner else {
            return;
        };
        if inner.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner = Arc::clone(inner);
        let _ = thread::Builder::new()
            .name("actrail-workload-diagnostics".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(inner.interval);
                    let snapshot = inner.counters.snapshot_and_reset();
                    tracing::info!(target: "actrail::workload", "workload {}", snapshot.format());
                }
            });
    }

    pub(crate) fn record_ready_cycle(
        &self,
        listener_ready: bool,
        control_ready_count: usize,
        event_source_ready: bool,
        background_ready: bool,
        event_fd_count: usize,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        let counters = &inner.counters;
        counters.ready_cycles.add(1);
        counters.control_ready.add(control_ready_count as u64);
        counters.event_fds_sampled.add(event_fd_count as u64);
        counters.event_fds_max.max(event_fd_count as u64);
        if listener_ready {
            counters.listener_ready.add(1);
        }
        if event_source_ready {
            counters.event_source_ready.add(1);
        }
        if background_ready {
            counters.background_ready.add(1);
        }
    }

    pub(crate) fn record_drain_call(&self, active_bindings: usize, active_path: bool) {
        let Some(inner) = &self.inner else {
            return;
        };
        let counters = &inner.counters;
        counters.drain_calls.add(1);
        counters.active_binding_samples.add(active_bindings as u64);
        counters.active_binding_max.max(active_bindings as u64);
        if active_path {
            counters.drain_active_path.add(1);
        } else {
            counters.drain_idle_path.add(1);
        }
    }

    pub(crate) fn record_drain_result(&self, elapsed: Duration, ok: bool) {
        let Some(inner) = &self.inner else {
            return;
        };
        let counters = &inner.counters;
        counters.drain_elapsed_max_us.max(duration_micros(elapsed));
        if !ok {
            counters.drain_errors.add(1);
        }
    }

    pub(crate) fn record_collector_batch(&self, observations: usize, payload_segments: usize) {
        let Some(inner) = &self.inner else {
            return;
        };
        let counters = &inner.counters;
        counters.collector_batches.add(1);
        counters.collector_observations.add(observations as u64);
        counters
            .collector_payload_segments
            .add(payload_segments as u64);
        if observations == 0 && payload_segments == 0 {
            counters.collector_empty_batches.add(1);
        }
    }

    pub(crate) fn record_payload_segments(&self, stage: PayloadSegmentStage, count: usize) {
        let Some(inner) = &self.inner else {
            return;
        };
        match stage {
            PayloadSegmentStage::TlsSync => inner.counters.tls_sync_segments.add(count as u64),
            PayloadSegmentStage::SeccompTls => {
                inner.counters.seccomp_tls_segments.add(count as u64)
            }
            PayloadSegmentStage::SeccompSocket => {
                inner.counters.seccomp_socket_segments.add(count as u64);
            }
        }
    }

    pub(crate) fn record_event_projection(
        &self,
        input_events: usize,
        retained_events: usize,
        semantic_actions: usize,
        semantic_links: usize,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        let counters = &inner.counters;
        counters.projected_input_events.add(input_events as u64);
        counters.retained_events.add(retained_events as u64);
        counters.semantic_actions.add(semantic_actions as u64);
        counters.semantic_links.add(semantic_links as u64);
    }

    pub(crate) fn record_storage_batch(
        &self,
        elapsed: Duration,
        events: usize,
        payload_segments: usize,
        diagnostics: usize,
        semantic_actions: usize,
        semantic_links: usize,
        trace_states: usize,
        ok: bool,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        let counters = &inner.counters;
        counters.storage_batches.add(1);
        counters.storage_events.add(events as u64);
        counters
            .storage_payload_segments
            .add(payload_segments as u64);
        counters.storage_diagnostics.add(diagnostics as u64);
        counters
            .storage_semantic_actions
            .add(semantic_actions as u64);
        counters.storage_semantic_links.add(semantic_links as u64);
        counters.storage_trace_states.add(trace_states as u64);
        counters
            .storage_elapsed_max_us
            .max(duration_micros(elapsed));
        if !ok {
            counters.storage_errors.add(1);
        }
    }

    pub(crate) fn record_payload_transaction_phase(
        &self,
        phase: PayloadTransactionPhase,
        elapsed: Duration,
        output_items: usize,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        let counters = &inner.counters;
        match phase {
            PayloadTransactionPhase::RetentionCheck => {
                counters.payload_retention_checks.add(1);
                counters
                    .payload_retention_check_max_us
                    .max(duration_micros(elapsed));
            }
            PayloadTransactionPhase::SemanticObserve => {
                counters.payload_semantic_observes.add(1);
                counters
                    .payload_semantic_observe_max_us
                    .max(duration_micros(elapsed));
            }
            PayloadTransactionPhase::SegmentPersist => {
                counters.payload_segment_persists.add(1);
                counters
                    .payload_segment_persist_max_us
                    .max(duration_micros(elapsed));
            }
            PayloadTransactionPhase::ApplicationAnalyze => {
                counters.payload_application_analyzes.add(1);
                counters
                    .payload_application_analyze_outputs
                    .add(output_items as u64);
                counters
                    .payload_application_analyze_max_us
                    .max(duration_micros(elapsed));
            }
            PayloadTransactionPhase::ApplicationPersist => {
                counters
                    .payload_application_persists
                    .add(output_items as u64);
                counters
                    .payload_application_persist_max_us
                    .max(duration_micros(elapsed));
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum PayloadSegmentStage {
    TlsSync,
    SeccompTls,
    SeccompSocket,
}

#[derive(Clone, Copy)]
pub(crate) enum PayloadTransactionPhase {
    RetentionCheck,
    SemanticObserve,
    SegmentPersist,
    ApplicationAnalyze,
    ApplicationPersist,
}

struct WorkloadDiagnosticsInner {
    interval: Duration,
    started: AtomicBool,
    counters: WorkloadCounters,
}

#[derive(Default)]
struct WorkloadCounters {
    ready_cycles: Counter,
    listener_ready: Counter,
    control_ready: Counter,
    event_source_ready: Counter,
    background_ready: Counter,
    event_fds_sampled: Counter,
    event_fds_max: Counter,
    drain_calls: Counter,
    drain_active_path: Counter,
    drain_idle_path: Counter,
    drain_errors: Counter,
    drain_elapsed_max_us: Counter,
    active_binding_samples: Counter,
    active_binding_max: Counter,
    collector_batches: Counter,
    collector_empty_batches: Counter,
    collector_observations: Counter,
    collector_payload_segments: Counter,
    tls_sync_segments: Counter,
    seccomp_tls_segments: Counter,
    seccomp_socket_segments: Counter,
    projected_input_events: Counter,
    retained_events: Counter,
    semantic_actions: Counter,
    semantic_links: Counter,
    storage_batches: Counter,
    storage_events: Counter,
    storage_payload_segments: Counter,
    storage_diagnostics: Counter,
    storage_semantic_actions: Counter,
    storage_semantic_links: Counter,
    storage_trace_states: Counter,
    storage_errors: Counter,
    storage_elapsed_max_us: Counter,
    payload_retention_checks: Counter,
    payload_retention_check_max_us: Counter,
    payload_semantic_observes: Counter,
    payload_semantic_observe_max_us: Counter,
    payload_segment_persists: Counter,
    payload_segment_persist_max_us: Counter,
    payload_application_analyzes: Counter,
    payload_application_analyze_outputs: Counter,
    payload_application_analyze_max_us: Counter,
    payload_application_persists: Counter,
    payload_application_persist_max_us: Counter,
}

impl WorkloadCounters {
    fn snapshot_and_reset(&self) -> WorkloadSnapshot {
        WorkloadSnapshot {
            ready_cycles: self.ready_cycles.take(),
            listener_ready: self.listener_ready.take(),
            control_ready: self.control_ready.take(),
            event_source_ready: self.event_source_ready.take(),
            background_ready: self.background_ready.take(),
            event_fds_sampled: self.event_fds_sampled.take(),
            event_fds_max: self.event_fds_max.take(),
            drain_calls: self.drain_calls.take(),
            drain_active_path: self.drain_active_path.take(),
            drain_idle_path: self.drain_idle_path.take(),
            drain_errors: self.drain_errors.take(),
            drain_elapsed_max_us: self.drain_elapsed_max_us.take(),
            active_binding_samples: self.active_binding_samples.take(),
            active_binding_max: self.active_binding_max.take(),
            collector_batches: self.collector_batches.take(),
            collector_empty_batches: self.collector_empty_batches.take(),
            collector_observations: self.collector_observations.take(),
            collector_payload_segments: self.collector_payload_segments.take(),
            tls_sync_segments: self.tls_sync_segments.take(),
            seccomp_tls_segments: self.seccomp_tls_segments.take(),
            seccomp_socket_segments: self.seccomp_socket_segments.take(),
            projected_input_events: self.projected_input_events.take(),
            retained_events: self.retained_events.take(),
            semantic_actions: self.semantic_actions.take(),
            semantic_links: self.semantic_links.take(),
            storage_batches: self.storage_batches.take(),
            storage_events: self.storage_events.take(),
            storage_payload_segments: self.storage_payload_segments.take(),
            storage_diagnostics: self.storage_diagnostics.take(),
            storage_semantic_actions: self.storage_semantic_actions.take(),
            storage_semantic_links: self.storage_semantic_links.take(),
            storage_trace_states: self.storage_trace_states.take(),
            storage_errors: self.storage_errors.take(),
            storage_elapsed_max_us: self.storage_elapsed_max_us.take(),
            payload_retention_checks: self.payload_retention_checks.take(),
            payload_retention_check_max_us: self.payload_retention_check_max_us.take(),
            payload_semantic_observes: self.payload_semantic_observes.take(),
            payload_semantic_observe_max_us: self.payload_semantic_observe_max_us.take(),
            payload_segment_persists: self.payload_segment_persists.take(),
            payload_segment_persist_max_us: self.payload_segment_persist_max_us.take(),
            payload_application_analyzes: self.payload_application_analyzes.take(),
            payload_application_analyze_outputs: self.payload_application_analyze_outputs.take(),
            payload_application_analyze_max_us: self.payload_application_analyze_max_us.take(),
            payload_application_persists: self.payload_application_persists.take(),
            payload_application_persist_max_us: self.payload_application_persist_max_us.take(),
        }
    }
}

#[derive(Default)]
struct Counter(AtomicU64);

impl Counter {
    fn add(&self, value: u64) {
        self.0.fetch_add(value, Ordering::Relaxed);
    }

    fn max(&self, value: u64) {
        self.0.fetch_max(value, Ordering::Relaxed);
    }

    fn take(&self) -> u64 {
        self.0.swap(0, Ordering::Relaxed)
    }
}

struct WorkloadSnapshot {
    ready_cycles: u64,
    listener_ready: u64,
    control_ready: u64,
    event_source_ready: u64,
    background_ready: u64,
    event_fds_sampled: u64,
    event_fds_max: u64,
    drain_calls: u64,
    drain_active_path: u64,
    drain_idle_path: u64,
    drain_errors: u64,
    drain_elapsed_max_us: u64,
    active_binding_samples: u64,
    active_binding_max: u64,
    collector_batches: u64,
    collector_empty_batches: u64,
    collector_observations: u64,
    collector_payload_segments: u64,
    tls_sync_segments: u64,
    seccomp_tls_segments: u64,
    seccomp_socket_segments: u64,
    projected_input_events: u64,
    retained_events: u64,
    semantic_actions: u64,
    semantic_links: u64,
    storage_batches: u64,
    storage_events: u64,
    storage_payload_segments: u64,
    storage_diagnostics: u64,
    storage_semantic_actions: u64,
    storage_semantic_links: u64,
    storage_trace_states: u64,
    storage_errors: u64,
    storage_elapsed_max_us: u64,
    payload_retention_checks: u64,
    payload_retention_check_max_us: u64,
    payload_semantic_observes: u64,
    payload_semantic_observe_max_us: u64,
    payload_segment_persists: u64,
    payload_segment_persist_max_us: u64,
    payload_application_analyzes: u64,
    payload_application_analyze_outputs: u64,
    payload_application_analyze_max_us: u64,
    payload_application_persists: u64,
    payload_application_persist_max_us: u64,
}

impl WorkloadSnapshot {
    fn format(&self) -> String {
        format!(
            concat!(
                "ready_cycles={} listener_ready={} control_ready={} ",
                "event_source_ready={} background_ready={} event_fds_sampled={} event_fds_max={} ",
                "drain_calls={} drain_active_path={} drain_idle_path={} drain_errors={} ",
                "drain_elapsed_max_us={} active_binding_samples={} active_binding_max={} ",
                "collector_batches={} collector_empty_batches={} collector_observations={} ",
                "collector_payload_segments={} tls_sync_segments={} seccomp_tls_segments={} ",
                "seccomp_socket_segments={} projected_input_events={} retained_events={} ",
                "semantic_actions={} semantic_links={} storage_batches={} storage_events={} ",
                "storage_payload_segments={} storage_diagnostics={} storage_semantic_actions={} ",
                "storage_semantic_links={} storage_trace_states={} storage_errors={} ",
                "storage_elapsed_max_us={} payload_retention_checks={} ",
                "payload_retention_check_max_us={} payload_semantic_observes={} ",
                "payload_semantic_observe_max_us={} payload_segment_persists={} ",
                "payload_segment_persist_max_us={} payload_application_analyzes={} ",
                "payload_application_analyze_outputs={} payload_application_analyze_max_us={} ",
                "payload_application_persists={} payload_application_persist_max_us={}"
            ),
            self.ready_cycles,
            self.listener_ready,
            self.control_ready,
            self.event_source_ready,
            self.background_ready,
            self.event_fds_sampled,
            self.event_fds_max,
            self.drain_calls,
            self.drain_active_path,
            self.drain_idle_path,
            self.drain_errors,
            self.drain_elapsed_max_us,
            self.active_binding_samples,
            self.active_binding_max,
            self.collector_batches,
            self.collector_empty_batches,
            self.collector_observations,
            self.collector_payload_segments,
            self.tls_sync_segments,
            self.seccomp_tls_segments,
            self.seccomp_socket_segments,
            self.projected_input_events,
            self.retained_events,
            self.semantic_actions,
            self.semantic_links,
            self.storage_batches,
            self.storage_events,
            self.storage_payload_segments,
            self.storage_diagnostics,
            self.storage_semantic_actions,
            self.storage_semantic_links,
            self.storage_trace_states,
            self.storage_errors,
            self.storage_elapsed_max_us,
            self.payload_retention_checks,
            self.payload_retention_check_max_us,
            self.payload_semantic_observes,
            self.payload_semantic_observe_max_us,
            self.payload_segment_persists,
            self.payload_segment_persist_max_us,
            self.payload_application_analyzes,
            self.payload_application_analyze_outputs,
            self.payload_application_analyze_max_us,
            self.payload_application_persists,
            self.payload_application_persist_max_us
        )
    }
}

fn duration_micros(duration: Duration) -> u64 {
    duration.as_micros().try_into().unwrap_or(u64::MAX)
}

pub(crate) fn now() -> Instant {
    Instant::now()
}
