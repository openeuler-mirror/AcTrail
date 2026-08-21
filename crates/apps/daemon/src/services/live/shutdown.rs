//! Trace shutdown draining before root removal and collector unbind.

use std::time::{Duration, Instant, SystemTime};

use collector_instance::CollectorInstance;
use config_core::daemon::DiagnosticLogLevel;
use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::ids::TraceId;
use model_core::process::MembershipState;
use model_core::trace::TraceLifecycleState;
use recording_runtime::RecordingWriter;
use trace_runtime::commands::RootRemovalRequest;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;

use super::{RuntimeDropDiagnosticDraft, warn_best_effort};

impl StorageAttachService {
    pub(in crate::services) fn shutdown_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let shutdown_started_at = Instant::now();
        let shutdown_deadline =
            ShutdownDeadline::new(shutdown_started_at, self.shutdown_runtime_timeout);
        let mut failures = Vec::new();
        let terminal_budget =
            shutdown_deadline.remaining_budget(self.finalization_shutdown_drain_timeout);
        let terminal_probe = self.begin_shutdown_stage_probe(
            ShutdownDrainStage::TerminalFinalization,
            terminal_budget,
            self.terminal_trace_count(trace_runtime),
            shutdown_started_at,
        );
        let terminal_result =
            self.drain_terminal_finalizations_for_shutdown(trace_runtime, terminal_budget);
        self.finish_shutdown_stage_probe(
            terminal_probe,
            self.terminal_trace_count(trace_runtime),
            terminal_result.as_ref().err(),
        );
        if let Err(error) = terminal_result {
            failures.push(format!(
                "terminal finalization: {}: {}",
                error.code, error.message
            ));
        }
        let unsettled_budget =
            shutdown_deadline.remaining_budget(self.finalization_shutdown_drain_timeout);
        let unsettled_probe = self.begin_shutdown_stage_probe(
            ShutdownDrainStage::UnsettledSemantics,
            unsettled_budget,
            self.unsettled_semantic_trace_count(trace_runtime),
            shutdown_started_at,
        );
        let unsettled_result =
            self.finalize_unsettled_semantics_for_shutdown(trace_runtime, unsettled_budget);
        self.finish_shutdown_stage_probe(
            unsettled_probe,
            self.unsettled_semantic_trace_count(trace_runtime),
            unsettled_result.as_ref().err(),
        );
        if let Err(error) = unsettled_result {
            failures.push(format!(
                "unsettled semantic finalization: {}: {}",
                error.code, error.message
            ));
        }
        let post_trace_budget = shutdown_deadline
            .remaining_budget(self.post_trace_coordinator.shutdown_drain_timeout());
        let post_trace_probe = self.begin_shutdown_stage_probe(
            ShutdownDrainStage::PostTrace,
            post_trace_budget,
            self.post_trace_coordinator.running_instance_ids().len(),
            shutdown_started_at,
        );
        let post_trace_result = self.shutdown_post_trace_runtime_with_timeout(post_trace_budget);
        self.finish_shutdown_stage_probe(
            post_trace_probe,
            self.post_trace_coordinator.running_instance_ids().len(),
            post_trace_result.as_ref().err(),
        );
        if let Err(error) = post_trace_result {
            failures.push(format!(
                "post-trace drain: {}: {}",
                error.code, error.message
            ));
        }
        let export_budget =
            shutdown_deadline.remaining_budget(self.finalization_shutdown_drain_timeout);
        let export_probe = self.begin_shutdown_stage_probe(
            ShutdownDrainStage::Export,
            export_budget,
            self.export_runtime.plugin_statuses().len(),
            shutdown_started_at,
        );
        let export_result = if export_budget.is_zero() {
            Err(shutdown_deadline_exhausted(ShutdownDrainStage::Export))
        } else {
            let export_drop_report = self
                .export_runtime
                .shutdown_observation_consumers(export_budget);
            let result = self.persist_export_drop_report(export_drop_report);
            if shutdown_deadline.is_expired() {
                Err(shutdown_deadline_exhausted(ShutdownDrainStage::Export))
            } else {
                result
            }
        };
        self.finish_shutdown_stage_probe(export_probe, 0, export_result.as_ref().err());
        if let Err(error) = export_result {
            failures.push(format!(
                "observation exporter drain: {}: {}",
                error.code, error.message
            ));
        }
        let alert_budget = shutdown_deadline.remaining_budget(self.alert_ingress.drain_timeout());
        let alert_probe = self.begin_shutdown_stage_probe(
            ShutdownDrainStage::Alert,
            alert_budget,
            self.shutdown_alert_remaining_count(),
            shutdown_started_at,
        );
        let alert_result = self.shutdown_alert_ingress_with_timeout(alert_budget);
        self.finish_shutdown_stage_probe(
            alert_probe,
            self.shutdown_alert_remaining_count(),
            alert_result.as_ref().err(),
        );
        if let Err(error) = alert_result {
            failures.push(format!("alert drain: {}: {}", error.code, error.message));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ControlError::new("daemon_shutdown", failures.join("; ")))
        }
    }

    fn unsettled_semantic_trace_count(&self, trace_runtime: &TraceRuntime) -> usize {
        trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| !self.post_trace_coordinator.barrier_ready(trace.trace_id))
            .count()
    }

    fn terminal_trace_count(&self, trace_runtime: &TraceRuntime) -> usize {
        trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| {
                trace.lifecycle_state.is_terminal()
                    && !self.finalized_terminal_traces.contains(&trace.trace_id)
            })
            .count()
    }

    fn shutdown_alert_remaining_count(&self) -> usize {
        usize::from(self.alert_ingress.has_outstanding_writes().unwrap_or(true))
    }

    fn begin_shutdown_stage_probe(
        &mut self,
        stage: ShutdownDrainStage,
        budget: Duration,
        remaining_count: usize,
        shutdown_started_at: Instant,
    ) -> ShutdownStageProbe {
        let mut probe = ShutdownStageProbe::new(stage, budget, shutdown_started_at);
        self.persist_shutdown_stage_diagnostic_fail_local(
            &probe,
            ShutdownStageStatus::Started,
            remaining_count,
        );
        probe.mark_stage_started();
        probe
    }

    fn finish_shutdown_stage_probe(
        &mut self,
        probe: ShutdownStageProbe,
        remaining_count: usize,
        error: Option<&ControlError>,
    ) {
        let status = if error.is_some() {
            ShutdownStageStatus::Failed
        } else {
            ShutdownStageStatus::Completed
        };
        self.persist_shutdown_stage_diagnostic_fail_local(&probe, status, remaining_count);
    }

    fn persist_shutdown_stage_diagnostic_fail_local(
        &mut self,
        probe: &ShutdownStageProbe,
        status: ShutdownStageStatus,
        remaining_count: usize,
    ) {
        let stage_elapsed = probe.stage_elapsed();
        let shutdown_elapsed = probe.shutdown_elapsed();
        let slow = stage_elapsed >= probe.slow_threshold();
        let diagnostic_id = match self.next_diagnostic_id() {
            Ok(diagnostic_id) => diagnostic_id,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    stage = probe.stage.as_str(),
                    "shutdown drain diagnostic id allocation failed; continuing shutdown"
                );
                return;
            }
        };
        let diagnostic = DiagnosticRecord::new(
            diagnostic_id,
            None,
            DiagnosticKind::RuntimeFailure,
            status.severity(slow),
            SystemTime::now(),
            status.message(),
        )
        .with_metadata("component", "daemon_shutdown")
        .with_metadata("code", status.code())
        .with_metadata("stage", probe.stage.as_str())
        .with_metadata("status", status.as_str())
        .with_metadata("slow", slow.to_string())
        .with_metadata("remaining_unit", probe.stage.remaining_unit())
        .with_metadata(
            "elapsed_ms",
            ShutdownStageProbe::duration_millis(stage_elapsed).to_string(),
        )
        .with_metadata(
            "shutdown_elapsed_ms",
            ShutdownStageProbe::duration_millis(shutdown_elapsed).to_string(),
        )
        .with_metadata(
            "budget_ms",
            ShutdownStageProbe::duration_millis(probe.budget).to_string(),
        )
        .with_metadata(
            "remaining_count",
            u64::try_from(remaining_count)
                .unwrap_or(u64::MAX)
                .to_string(),
        );
        if let Err(error) =
            RecordingWriter::new(self.storage.as_mut()).persist_diagnostic(diagnostic)
        {
            tracing::warn!(
                error = ?error,
                stage = probe.stage.as_str(),
                "shutdown drain diagnostic persistence failed; continuing shutdown"
            );
        }
    }

    fn finalize_unsettled_semantics_for_shutdown(
        &mut self,
        trace_runtime: &TraceRuntime,
        timeout: Duration,
    ) -> Result<(), ControlError> {
        let trace_ids = trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| !self.post_trace_coordinator.barrier_ready(trace.trace_id))
            .map(|trace| trace.trace_id)
            .collect::<Vec<_>>();
        let finished_at = SystemTime::now();
        let deadline = Instant::now().checked_add(timeout);
        let mut failures = Vec::new();
        for trace_id in trace_ids {
            if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                failures.push("global shutdown deadline exhausted".to_string());
                break;
            }
            if let Err(error) =
                self.finalize_semantic_projection_for_trace(trace_runtime, trace_id, finished_at)
            {
                failures.push(format!("{trace_id}: {}: {}", error.code, error.message));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ControlError::new(
                "shutdown_semantic_finalization",
                failures.join("; "),
            ))
        }
    }

    fn drain_terminal_finalizations_for_shutdown(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        timeout: Duration,
    ) -> Result<(), ControlError> {
        let started_at = Instant::now();
        let mut previous_pending_count = usize::MAX;
        loop {
            if self.pending_terminal_finalizations.is_empty()
                && self.terminal_trace_count(trace_runtime) == 0
            {
                return Ok(());
            }
            if started_at.elapsed() >= timeout {
                return self
                    .fail_shutdown_with_pending_finalizations(trace_runtime, started_at.elapsed());
            }
            self.drain_live_events_impl(trace_runtime)?;
            let pending_count = self.pending_terminal_finalizations.len();
            if pending_count == 0 {
                return Ok(());
            }
            let elapsed = started_at.elapsed();
            if elapsed >= timeout {
                return self.fail_shutdown_with_pending_finalizations(trace_runtime, elapsed);
            }
            if pending_count < previous_pending_count {
                previous_pending_count = pending_count;
                continue;
            }
            previous_pending_count = pending_count;
            let remaining = timeout.saturating_sub(elapsed);
            let sleep_for = self.finalization_poll_interval.min(remaining);
            if sleep_for.is_zero() {
                std::thread::yield_now();
            } else {
                std::thread::sleep(sleep_for);
            }
        }
    }

    fn fail_shutdown_with_pending_finalizations(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        elapsed: std::time::Duration,
    ) -> Result<(), ControlError> {
        let trace_ids = self
            .pending_terminal_finalizations
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let trace_list = trace_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let message = format!(
            "daemon shutdown left {} terminal trace(s) pending after {}ms: {}",
            trace_ids.len(),
            elapsed.as_millis(),
            trace_list
        );
        let mut trace_states = Vec::with_capacity(trace_ids.len());
        for trace_id in &trace_ids {
            trace_runtime.mark_degraded(*trace_id).map_err(|error| {
                ControlError::new(
                    "trace_finalization_shutdown_timeout",
                    format!("{message}; mark {trace_id} degraded failed: {error:?}"),
                )
            })?;
            trace_states.push(self.trace_state_record_for_persistence(trace_runtime, *trace_id)?);
        }
        let drafts = trace_ids
            .into_iter()
            .map(|trace_id| RuntimeDropDiagnosticDraft {
                trace_id: Some(trace_id),
                code: "trace_finalization_shutdown_timeout".to_string(),
                message: message.clone(),
            })
            .collect();
        self.persist_runtime_drop_diagnostics(trace_runtime, drafts, trace_states)
            .map_err(|error| {
                ControlError::new(
                    "trace_finalization_shutdown_timeout",
                    format!(
                        "{message}; persist timeout diagnostics failed: {}: {}",
                        error.code, error.message
                    ),
                )
            })?;
        Err(ControlError::new(
            "trace_finalization_shutdown_timeout",
            message,
        ))
    }

    pub(in crate::services) fn finalize_terminal_traces_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        self.enqueue_terminal_finalizations_impl(trace_runtime);
        self.progress_terminal_finalizations_impl(trace_runtime)
    }

    fn enqueue_terminal_finalizations_impl(&mut self, trace_runtime: &TraceRuntime) {
        for trace in trace_runtime.list_trace_records() {
            if trace.lifecycle_state.is_terminal()
                && !self.finalized_terminal_traces.contains(&trace.trace_id)
            {
                self.queue_terminal_finalization(trace.trace_id);
            }
        }
    }

    fn enqueue_trace_finalization_if_terminal(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
    ) -> Result<(), ControlError> {
        let trace = trace_runtime
            .get_trace(trace_id)
            .map(|entry| &entry.trace)
            .ok_or_else(|| ControlError::new("terminal_trace", "trace not found"))?;
        if trace.lifecycle_state.is_terminal()
            && !self.finalized_terminal_traces.contains(&trace_id)
        {
            self.queue_terminal_finalization(trace_id);
        }
        Ok(())
    }

    fn queue_terminal_finalization(&mut self, trace_id: TraceId) {
        self.pending_terminal_finalizations.insert(trace_id);
        self.terminal_finalization_queued_at
            .entry(trace_id)
            .or_insert_with(Instant::now);
    }

    fn progress_terminal_finalizations_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let trace_ids = self
            .pending_terminal_finalizations
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let mut finalized_this_cycle = 0_usize;

        for trace_id in trace_ids {
            if finalized_this_cycle >= self.finalization_traces_per_cycle {
                break;
            }
            if self.finalized_terminal_traces.contains(&trace_id) {
                self.pending_terminal_finalizations.remove(&trace_id);
                self.terminal_finalization_queued_at.remove(&trace_id);
                continue;
            }
            if !trace_is_terminal(trace_runtime, trace_id)? {
                self.pending_terminal_finalizations.remove(&trace_id);
                self.terminal_finalization_queued_at.remove(&trace_id);
                continue;
            }
            if self
                .terminal_finalization_queued_at
                .get(&trace_id)
                .is_some_and(|queued_at| queued_at.elapsed() < self.terminal_settle_delay)
            {
                continue;
            }
            if terminal_trace_has_open_memberships(trace_runtime, trace_id)? {
                continue;
            }
            if !self.post_trace_coordinator.barrier_ready(trace_id) {
                let finished_at = terminal_trace_finished_at(trace_runtime, trace_id)?;
                self.finalize_semantic_projection_for_trace(trace_runtime, trace_id, finished_at)?;
                self.collector
                    .unbind_trace(trace_id)
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
                self.post_trace_coordinator.mark_barrier_ready(trace_id);
            }
            let post_trace_instances = self.export_runtime.post_trace_instance_ids();
            let admission = self.post_trace_coordinator.admit_trace(
                trace_id,
                &post_trace_instances,
                &self.export_runtime,
                self.storage.as_mut(),
            )?;
            self.persist_post_trace_issues(admission.timeout_diagnostics)?;
            if !admission.all_admitted {
                continue;
            }
            // Admission only guarantees that every analyzer task reached its
            // plugin worker. Keep the trace finalization barrier in place until
            // all completions have been drained so post-trace host calls cannot
            // race with trace-runtime cleanup.
            if self
                .post_trace_coordinator
                .has_running_tasks_for_trace(trace_id)
            {
                continue;
            }
            self.application_protocol.forget_trace(trace_id);
            self.semantic_actions.forget_trace(trace_id);
            self.socket_payload_gate.forget_trace(trace_id);
            self.payload_body_retention_gate.forget_trace(trace_id);
            self.retained_payload_bytes_by_trace.remove(&trace_id);
            self.finalized_terminal_traces.insert(trace_id);
            self.pending_terminal_finalizations.remove(&trace_id);
            self.terminal_finalization_queued_at.remove(&trace_id);
            self.post_trace_coordinator.mark_trace_finalized(trace_id);
            warn_best_effort(
                self.network_control.forget_trace(trace_id),
                "network_control_forget_trace",
            );
            trace_runtime.forget_trace(trace_id);
            finalized_this_cycle += 1;
            self.log_diagnostic(
                DiagnosticLogLevel::Info,
                format_args!(
                    "trace_finalization completed trace_id={} post_trace_tasks={}",
                    trace_id,
                    post_trace_instances.len()
                ),
            );
        }
        Ok(())
    }

    pub(in crate::services) fn remove_root_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        trace_id: TraceId,
        removed_at: SystemTime,
    ) -> Result<(), ControlError> {
        let root_identity = trace_runtime
            .get_trace(trace_id)
            .map(|entry| entry.trace.root_process_identity.clone())
            .ok_or_else(|| ControlError::new("track_remove", "trace not found"))?;
        let root_host_pid = self
            .process_registry
            .record(root_identity)
            .and_then(|record| record.host.as_ref())
            .map(|host| host.pid)
            .ok_or_else(|| {
                ControlError::new("track_remove", "root process has no resolved host PID")
            })?;

        trace_runtime
            .track_remove_root(RootRemovalRequest {
                trace_id,
                removed_at,
            })
            .map_err(|error| ControlError::new("track_remove", format!("{:?}", error)))?;
        self.collector
            .stop_kernel_tracking_process(root_host_pid)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        self.persist_trace_state(trace_runtime, trace_id)?;
        self.enqueue_trace_finalization_if_terminal(trace_runtime, trace_id)?;
        self.pending_tool_names.remove(&trace_id);
        let finalization_state = if self.pending_terminal_finalizations.contains(&trace_id) {
            "queued"
        } else {
            "not_terminal"
        };
        self.log_diagnostic(
            DiagnosticLogLevel::Info,
            format_args!(
                "agent_launch root_removed trace_id={} process_id={} host_pid={} finalization={}",
                trace_id, root_identity, root_host_pid, finalization_state
            ),
        );
        Ok(())
    }
}

struct ShutdownDeadline {
    deadline: Option<Instant>,
}

impl ShutdownDeadline {
    fn new(started_at: Instant, timeout: Duration) -> Self {
        Self {
            deadline: started_at.checked_add(timeout),
        }
    }

    fn remaining_budget(&self, stage_limit: Duration) -> Duration {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_default()
            .min(stage_limit)
    }

    fn is_expired(&self) -> bool {
        self.deadline
            .is_none_or(|deadline| Instant::now() >= deadline)
    }
}

#[derive(Clone, Copy)]
enum ShutdownDrainStage {
    TerminalFinalization,
    UnsettledSemantics,
    PostTrace,
    Export,
    Alert,
}

fn shutdown_deadline_exhausted(stage: ShutdownDrainStage) -> ControlError {
    ControlError::new(
        "daemon_shutdown_deadline",
        format!(
            "global shutdown deadline exhausted before {} completed",
            stage.as_str()
        ),
    )
}

impl ShutdownDrainStage {
    const COUNT: u32 = 5;

    fn as_str(self) -> &'static str {
        match self {
            Self::TerminalFinalization => "terminal_finalization",
            Self::UnsettledSemantics => "unsettled_semantics",
            Self::PostTrace => "post_trace",
            Self::Export => "export",
            Self::Alert => "alert",
        }
    }

    fn remaining_unit(self) -> &'static str {
        match self {
            Self::TerminalFinalization | Self::UnsettledSemantics => "traces",
            Self::PostTrace => "plugin_instances",
            Self::Export => "observation_consumers",
            Self::Alert => "has_outstanding_writes",
        }
    }
}

struct ShutdownStageProbe {
    stage: ShutdownDrainStage,
    budget: Duration,
    shutdown_started_at: Instant,
    stage_started_at: Instant,
}

impl ShutdownStageProbe {
    fn new(stage: ShutdownDrainStage, budget: Duration, shutdown_started_at: Instant) -> Self {
        Self {
            stage,
            budget,
            shutdown_started_at,
            stage_started_at: Instant::now(),
        }
    }

    fn stage_elapsed(&self) -> Duration {
        self.stage_started_at.elapsed()
    }

    fn mark_stage_started(&mut self) {
        self.stage_started_at = Instant::now();
    }

    fn shutdown_elapsed(&self) -> Duration {
        self.shutdown_started_at.elapsed()
    }

    fn slow_threshold(&self) -> Duration {
        self.budget / ShutdownDrainStage::COUNT
    }

    fn duration_millis(duration: Duration) -> u64 {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    }
}

#[derive(Clone, Copy)]
enum ShutdownStageStatus {
    Started,
    Completed,
    Failed,
}

impl ShutdownStageStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Started => "daemon_shutdown_stage_started",
            Self::Completed => "daemon_shutdown_stage_completed",
            Self::Failed => "daemon_shutdown_stage_failed",
        }
    }

    fn severity(self, slow: bool) -> DiagnosticSeverity {
        match self {
            Self::Started => DiagnosticSeverity::Info,
            Self::Completed if slow => DiagnosticSeverity::Warning,
            Self::Completed => DiagnosticSeverity::Info,
            Self::Failed => DiagnosticSeverity::Error,
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Started => "daemon shutdown drain stage entered",
            Self::Completed => "daemon shutdown drain stage completed",
            Self::Failed => "daemon shutdown drain stage failed",
        }
    }
}

fn trace_is_terminal(
    trace_runtime: &TraceRuntime,
    trace_id: TraceId,
) -> Result<bool, ControlError> {
    trace_runtime
        .get_trace(trace_id)
        .map(|entry| entry.trace.lifecycle_state.is_terminal())
        .ok_or_else(|| ControlError::new("terminal_trace", "trace not found"))
}

fn terminal_trace_has_open_memberships(
    trace_runtime: &TraceRuntime,
    trace_id: TraceId,
) -> Result<bool, ControlError> {
    trace_runtime
        .get_trace(trace_id)
        .map(|entry| {
            entry.memberships.memberships().any(|membership| {
                membership.capture_enabled
                    && matches!(
                        membership.state,
                        MembershipState::Starting | MembershipState::Active
                    )
            })
        })
        .ok_or_else(|| ControlError::new("terminal_trace", "trace not found"))
}

fn terminal_trace_finished_at(
    trace_runtime: &TraceRuntime,
    trace_id: TraceId,
) -> Result<SystemTime, ControlError> {
    let trace = trace_runtime
        .get_trace(trace_id)
        .map(|entry| &entry.trace)
        .ok_or_else(|| ControlError::new("terminal_trace", "trace not found"))?;
    match trace.lifecycle_state {
        TraceLifecycleState::Completed => trace.timings.completed_at.ok_or_else(|| {
            ControlError::new("terminal_trace", "completed trace missing completed_at")
        }),
        TraceLifecycleState::Exited => trace
            .timings
            .exited_at
            .ok_or_else(|| ControlError::new("terminal_trace", "exited trace missing exited_at")),
        TraceLifecycleState::Failed => trace
            .timings
            .failed_at
            .ok_or_else(|| ControlError::new("terminal_trace", "failed trace missing failed_at")),
        _ => Err(ControlError::new("terminal_trace", "trace is not terminal")),
    }
}
