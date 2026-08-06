//! Trace shutdown draining before root removal and collector unbind.

use std::time::{Instant, SystemTime};

use collector_instance::CollectorInstance;
use config_core::daemon::DiagnosticLogLevel;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::process::MembershipState;
use model_core::trace::TraceLifecycleState;
use trace_runtime::commands::RootRemovalRequest;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;

use super::RuntimeDropDiagnosticDraft;

impl StorageAttachService {
    pub(in crate::services) fn shutdown_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let mut failures = Vec::new();
        if let Err(error) = self.drain_terminal_finalizations_for_shutdown(trace_runtime) {
            failures.push(format!(
                "terminal finalization: {}: {}",
                error.code, error.message
            ));
        }
        if let Err(error) = self.finalize_unsettled_semantics_for_shutdown(trace_runtime) {
            failures.push(format!(
                "unsettled semantic finalization: {}: {}",
                error.code, error.message
            ));
        }
        if let Err(error) = self.shutdown_post_trace_runtime_impl() {
            failures.push(format!(
                "post-trace drain: {}: {}",
                error.code, error.message
            ));
        }
        let export_drop_report = self.export_runtime.shutdown_observation_consumers();
        if let Err(error) = self.persist_export_drop_report(export_drop_report) {
            failures.push(format!(
                "observation exporter drain: {}: {}",
                error.code, error.message
            ));
        }
        if let Err(error) = self.shutdown_alert_ingress_impl() {
            failures.push(format!("alert drain: {}: {}", error.code, error.message));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ControlError::new("daemon_shutdown", failures.join("; ")))
        }
    }

    fn finalize_unsettled_semantics_for_shutdown(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        let trace_ids = trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| !self.post_trace_coordinator.barrier_ready(trace.trace_id))
            .map(|trace| trace.trace_id)
            .collect::<Vec<_>>();
        let finished_at = SystemTime::now();
        let mut failures = Vec::new();
        for trace_id in trace_ids {
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
    ) -> Result<(), ControlError> {
        let started_at = Instant::now();
        let mut previous_pending_count = usize::MAX;
        loop {
            self.drain_live_events_impl(trace_runtime)?;
            let pending_count = self.pending_terminal_finalizations.len();
            if pending_count == 0 {
                return Ok(());
            }
            let elapsed = started_at.elapsed();
            if elapsed >= self.finalization_shutdown_drain_timeout {
                return self.fail_shutdown_with_pending_finalizations(trace_runtime, elapsed);
            }
            if pending_count < previous_pending_count {
                previous_pending_count = pending_count;
                continue;
            }
            previous_pending_count = pending_count;
            let remaining = self
                .finalization_shutdown_drain_timeout
                .saturating_sub(elapsed);
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
