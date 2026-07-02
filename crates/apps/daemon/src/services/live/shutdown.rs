//! Trace shutdown draining before root removal and collector unbind.

use std::time::SystemTime;

use collector_instance::CollectorInstance;
use config_core::daemon::DiagnosticLogLevel;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::process::MembershipState;
use model_core::trace::TraceLifecycleState;
use trace_runtime::commands::RootRemovalRequest;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;

impl StorageAttachService {
    pub(in crate::services) fn finalize_terminal_traces_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        self.enqueue_terminal_finalizations_impl(trace_runtime);
        self.progress_terminal_finalizations_impl(trace_runtime)
    }

    fn enqueue_terminal_finalizations_impl(&mut self, trace_runtime: &TraceRuntime) {
        for trace in trace_runtime.list_trace_records() {
            if trace.lifecycle_state.is_terminal()
                && !self.finalized_terminal_traces.contains(&trace.trace_id)
            {
                self.pending_terminal_finalizations.insert(trace.trace_id);
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
            self.pending_terminal_finalizations.insert(trace_id);
        }
        Ok(())
    }

    fn progress_terminal_finalizations_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
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
                continue;
            }
            if !trace_is_terminal(trace_runtime, trace_id)? {
                self.pending_terminal_finalizations.remove(&trace_id);
                continue;
            }
            if terminal_trace_has_open_memberships(trace_runtime, trace_id)? {
                continue;
            }
            let finished_at = terminal_trace_finished_at(trace_runtime, trace_id)?;
            self.finalize_semantic_projection_for_trace(trace_runtime, trace_id, finished_at)?;
            self.collector
                .unbind_trace(trace_id)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            self.application_protocol.forget_trace(trace_id);
            self.semantic_actions.forget_trace(trace_id);
            self.socket_payload_gate.forget_trace(trace_id);
            self.payload_body_retention_gate.forget_trace(trace_id);
            self.retained_payload_bytes_by_trace.remove(&trace_id);
            self.finalized_terminal_traces.insert(trace_id);
            self.pending_terminal_finalizations.remove(&trace_id);
            finalized_this_cycle += 1;
            self.log_diagnostic(
                DiagnosticLogLevel::Info,
                format_args!("trace_finalization completed trace_id={trace_id}"),
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

        trace_runtime
            .track_remove_root(RootRemovalRequest {
                trace_id,
                removed_at,
            })
            .map_err(|error| ControlError::new("track_remove", format!("{:?}", error)))?;
        self.collector
            .stop_kernel_tracking_process(root_identity.pid)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        self.persist_trace_state(trace_runtime, trace_id)?;
        self.enqueue_trace_finalization_if_terminal(trace_runtime, trace_id)?;
        let finalization_state = if self.pending_terminal_finalizations.contains(&trace_id) {
            "queued"
        } else {
            "not_terminal"
        };
        self.log_diagnostic(
            DiagnosticLogLevel::Info,
            format_args!(
                "agent_launch root_removed trace_id={} pid={} generation={} finalization={}",
                trace_id, root_identity.pid, root_identity.generation, finalization_state
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
