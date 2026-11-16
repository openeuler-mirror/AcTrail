use crate::services::attach::StorageAttachService;
use control_contract::reply::ControlError;
use model_core::diagnostics::DiagnosticKind;
use model_core::diagnostics::DiagnosticRecord;
use model_core::diagnostics::DiagnosticSeverity;
use model_core::ids::TraceId;
use model_core::trace::TraceHealth;
use model_core::trace::TraceLifecycleState;
use recording_runtime::RecordingWriter;
use resource_profiling::model::resource_scope::ResourceScopeLifecycleState;
use std::time::SystemTime;
use trace_runtime::TraceRuntime;

impl StorageAttachService {
    pub(in crate::services) fn drain_resource_metrics_impl(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let drain = match self.resource_metrics.drain_due(
            trace_runtime,
            &self.process_registry,
            self.storage.as_mut(),
        ) {
            Ok(d) => d,
            Err(error) => {
                tracing::warn!(
                    error.code = %error.code,
                    error.message = %error.message,
                    "resource metrics sampling failed; skipping this cycle"
                );
                return Ok(());
            }
        };
        for failure in drain.failures {
            if failure.recovered {
                self.storage
                    .update_trace_health(failure.trace_id, TraceHealth::Degraded)
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
            } else {
                trace_runtime
                    .mark_degraded(failure.trace_id)
                    .map_err(|error| {
                        ControlError::new("resource_metrics_degraded", format!("{error:?}"))
                    })?;
            }
            let diagnostic = DiagnosticRecord::new(
                self.next_diagnostic_id()?,
                Some(failure.trace_id),
                DiagnosticKind::RuntimeFailure,
                DiagnosticSeverity::Warning,
                SystemTime::now(),
                "verified cgroup resource accounting unavailable",
            )
            .with_metadata("error", failure.message);
            RecordingWriter::new(self.storage.as_mut())
                .persist_diagnostic(diagnostic)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            if !failure.recovered {
                self.persist_trace_state(trace_runtime, failure.trace_id)?;
            }
        }
        if let Err(error) = self.drain_resource_finalizations_impl(trace_runtime) {
            tracing::warn!(message=%error.message,"managed resource finalization failed; retrying");
        }
        self.drain_v1_binding_finalizations(trace_runtime);
        if let Err(error) = self.drain_v2_binding_finalizations_impl(trace_runtime) {
            tracing::warn!(message=%error.message,"external resource finalization failed; retrying");
        }
        Ok(())
    }

    pub(in crate::services) fn drain_v2_binding_finalizations_impl(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let drafts = self.resource_metrics.poll_v2_binding_finalizations(
            trace_runtime,
            &self.process_registry,
            self.storage.as_mut(),
        )?;
        for draft in drafts {
            let trace_id = draft;
            if let Err(error) = self.persist_v2_binding_finalization(draft) {
                tracing::warn!(%trace_id,message=%error.message,"external control finalization failed; retrying");
            }
        }
        Ok(())
    }

    fn persist_v2_binding_finalization(&mut self, trace_id: TraceId) -> Result<(), ControlError> {
        self.storage
            .close_cgroup_v2_binding(trace_id, SystemTime::now())
            .map_err(|e| ControlError::new(e.stage, e.message))?;
        self.resource_metrics
            .finish_v2_binding_finalization(trace_id);
        Ok(())
    }

    pub(in crate::services) fn drain_resource_finalizations_impl(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let poll = self
            .resource_metrics
            .poll_finalizations(trace_runtime, &self.process_registry)?;
        for trace_id in poll.waiting {
            if let Err(error) = self.persist_resource_waiting(trace_id) {
                tracing::warn!(%trace_id,message=%error.message,"resource waiting state persistence failed; retrying");
            }
        }

        for finalization in poll.ready {
            let trace_id = finalization.trace_id;
            if let Err(error) = self.persist_managed_finalization(trace_runtime, finalization) {
                tracing::warn!(%trace_id,message=%error.message,"managed control finalization failed; retrying");
            }
        }
        Ok(())
    }

    fn persist_managed_finalization(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        finalization: resource_profiling::ResourceCompletion,
    ) -> Result<(), ControlError> {
        let lifecycle_state = if finalization.timed_out {
            ResourceScopeLifecycleState::Orphaned
        } else {
            ResourceScopeLifecycleState::Finalized
        };
        let (persisted_state, completed_at) =
            self.persist_resource_final_state(finalization.trace_id, lifecycle_state)?;
        let timed_out = persisted_state == ResourceScopeLifecycleState::Orphaned;
        if finalization.recovered {
            self.complete_recovered_resource_trace(finalization.trace_id, timed_out, completed_at)?;
            self.resource_metrics
                .finish_finalization(finalization.trace_id, !timed_out);
            return Ok(());
        }
        if timed_out {
            trace_runtime
                .mark_degraded(finalization.trace_id)
                .map_err(|error| {
                    ControlError::new("resource_finalization_degraded", format!("{error:?}"))
                })?;
        }
        let has_open_memberships = trace_runtime
            .complete_after_resource_barrier(finalization.trace_id, completed_at)
            .map_err(|error| {
                ControlError::new("resource_finalization_transition", format!("{error:?}"))
            })?;
        if has_open_memberships {
            self.persist_resource_open_membership_diagnostic(trace_runtime, finalization.trace_id)?;
        }
        self.persist_trace_state(trace_runtime, finalization.trace_id)?;
        self.resource_metrics
            .finish_finalization(finalization.trace_id, !timed_out);
        Ok(())
    }

    fn complete_recovered_resource_trace(
        &mut self,
        trace_id: TraceId,
        timed_out: bool,
        completed_at: SystemTime,
    ) -> Result<(), ControlError> {
        let mut trace = self
            .storage
            .get_trace(trace_id)
            .map_err(|error| ControlError::new(error.stage, error.message))?
            .ok_or_else(|| {
                ControlError::new("resource_recovery", "recovered trace record is missing")
            })?;
        trace.lifecycle_state = TraceLifecycleState::Completed;
        trace.timings.completed_at = Some(completed_at);
        if timed_out {
            trace.health = TraceHealth::Degraded;
        }
        self.storage
            .create_trace(trace)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let diagnostic = DiagnosticRecord::new(
            self.next_diagnostic_id()?,
            Some(trace_id),
            DiagnosticKind::RuntimeFailure,
            if timed_out {
                DiagnosticSeverity::Warning
            } else {
                DiagnosticSeverity::Info
            },
            completed_at,
            "resource scope finalized after daemon restart",
        )
        .with_metadata("timed_out", timed_out.to_string());
        RecordingWriter::new(self.storage.as_mut())
            .persist_diagnostic(diagnostic)
            .map_err(|error| ControlError::new(error.stage, error.message))
    }

    fn persist_resource_final_state(
        &mut self,
        trace_id: TraceId,
        lifecycle_state: ResourceScopeLifecycleState,
    ) -> Result<(ResourceScopeLifecycleState, SystemTime), ControlError> {
        if let Some(existing) = self
            .storage
            .get_resource_scope(trace_id)
            .map_err(|e| ControlError::new(e.stage, e.message))?
            .filter(|scope| !scope.lifecycle_state.is_live())
        {
            return Ok((existing.lifecycle_state, existing.updated_at));
        }
        let completed_at = SystemTime::now();
        self.storage
            .update_resource_scope_state(trace_id, lifecycle_state, completed_at)
            .map_err(|e| ControlError::new(e.stage, e.message))?;
        Ok((lifecycle_state, completed_at))
    }

    fn persist_resource_open_membership_diagnostic(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
    ) -> Result<(), ControlError> {
        let root = trace_runtime
            .get_trace(trace_id)
            .map(|entry| entry.trace.root_process_identity)
            .ok_or_else(|| ControlError::new("resource_finalization", "trace not found"))?;
        if !self
            .diagnosed_terminal_open_memberships
            .insert((trace_id, root))
        {
            return Ok(());
        }
        let diagnostic = DiagnosticRecord::new(
            self.next_diagnostic_id()?,
            Some(trace_id),
            DiagnosticKind::RuntimeFailure,
            DiagnosticSeverity::Warning,
            SystemTime::now(),
            "resource_finalized_with_open_memberships",
        )
        .with_process(root);
        RecordingWriter::new(self.storage.as_mut())
            .persist_diagnostic(diagnostic)
            .map_err(|error| ControlError::new(error.stage, error.message))
    }

    fn persist_resource_waiting(&mut self, id: TraceId) -> Result<(), ControlError> {
        let scope = self
            .storage
            .get_resource_scope(id)
            .map_err(|e| ControlError::new(e.stage, e.message))?
            .ok_or_else(|| ControlError::new("resource_waiting", "scope record missing"))?;
        if scope.lifecycle_state == ResourceScopeLifecycleState::Active {
            self.storage
                .update_resource_scope_state(
                    id,
                    ResourceScopeLifecycleState::WaitingForEmpty,
                    SystemTime::now(),
                )
                .map_err(|e| ControlError::new(e.stage, e.message))?;
        }
        Ok(())
    }
}
