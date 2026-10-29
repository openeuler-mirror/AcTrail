use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v2_binding::CgroupV2Binding;
use resource_profiling::model::cgroup_v2_binding::CgroupV2BindingStaleReason;
use resource_profiling::{CgroupV2BindingPersistence, CgroupV2BindingRecovery, ProfilingError};
use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;
use storage_core::StorageBackend;
pub(super) struct CgroupV2BindingStorage<'a> {
    pub storage: &'a mut dyn StorageBackend,
}
impl CgroupV2BindingStorage<'_> {
    fn storage_error(error: storage_core::StorageError) -> ProfilingError {
        ProfilingError::new(error.stage, error.message)
    }
}
impl CgroupV2BindingPersistence for CgroupV2BindingStorage<'_> {
    fn prepare_v2_binding_recovery(
        &mut self,
    ) -> Result<Vec<CgroupV2BindingRecovery>, ProfilingError> {
        let mut bindings = self
            .storage
            .list_live_cgroup_v2_bindings()
            .map_err(Self::storage_error)?;
        let mut recovered_processes = BTreeMap::new();
        let mut recovered_terminal = BTreeSet::new();
        for binding in &bindings {
            let Some(trace) = self
                .storage
                .get_trace(binding.trace_id)
                .map_err(Self::storage_error)?
            else {
                use model_core::diagnostics::{
                    DiagnosticKind, DiagnosticRecord, DiagnosticSeverity,
                };
                let diagnostic = DiagnosticRecord::new(
                    model_core::ids::DiagnosticId::new(
                        self.storage
                            .next_diagnostic_id_seed()
                            .map_err(Self::storage_error)?,
                    ),
                    None,
                    DiagnosticKind::RuntimeFailure,
                    DiagnosticSeverity::Warning,
                    SystemTime::now(),
                    format!(
                        "discarded interrupted external admission for missing trace {}",
                        binding.trace_id
                    ),
                );
                let transaction = self.storage.begin().map_err(Self::storage_error)?;
                let result = self.storage.append_diagnostic(diagnostic).and_then(|_| {
                    self.storage
                        .discard_orphan_cgroup_v2_binding(binding.trace_id)
                });
                if let Err(error) = result {
                    transaction.rollback().map_err(Self::storage_error)?;
                    return Err(Self::storage_error(error));
                }
                transaction.commit().map_err(Self::storage_error)?;
                tracing::warn!(trace_id = %binding.trace_id, "discarded interrupted external admission without trace");
                continue;
            };
            recovered_processes.insert(binding.trace_id, trace.root_process_identity);
            if trace.lifecycle_state.is_terminal() {
                recovered_terminal.insert(binding.trace_id);
            }
        }
        bindings.retain(|binding| recovered_processes.contains_key(&binding.trace_id));
        Ok(bindings
            .into_iter()
            .map(|binding| CgroupV2BindingRecovery {
                root: recovered_processes[&binding.trace_id],
                terminal: recovered_terminal.contains(&binding.trace_id),
                binding,
            })
            .collect())
    }
    fn record_v2_binding_success(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        self.storage
            .record_cgroup_v2_binding_success(id, at)
            .map_err(Self::storage_error)
    }
    fn record_v2_binding_failure(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<u32, ProfilingError> {
        self.storage
            .record_cgroup_v2_binding_failure(id, at)
            .map_err(Self::storage_error)
    }
    fn mark_v2_binding_stale(
        &mut self,
        id: TraceId,
        reason: CgroupV2BindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        self.storage
            .mark_cgroup_v2_binding_stale(id, reason, at)
            .map_err(Self::storage_error)
    }
    fn v2_binding(&self, id: TraceId) -> Result<Option<CgroupV2Binding>, ProfilingError> {
        self.storage
            .get_cgroup_v2_binding(id)
            .map_err(Self::storage_error)
    }
    fn complete_v2_binding_recovered_trace(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        let mut trace = self
            .storage
            .get_trace(id)
            .map_err(Self::storage_error)?
            .ok_or_else(|| ProfilingError::new("external_recovery", "binding has no trace"))?;
        if !trace.lifecycle_state.is_terminal() {
            trace.lifecycle_state = model_core::trace::TraceLifecycleState::Completed;
            trace.timings.completed_at = Some(at);
            trace.health = model_core::trace::TraceHealth::Degraded;
            self.storage
                .create_trace(trace)
                .map_err(Self::storage_error)?;
        }
        Ok(())
    }

    fn mark_v2_binding_recovery_stale(
        &mut self,
        trace_id: TraceId,
        reason: CgroupV2BindingStaleReason,
        now: SystemTime,
    ) -> Result<(), ProfilingError> {
        use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
        use model_core::trace::TraceHealth;
        let already_diagnosed = self
            .storage
            .list_diagnostics(trace_id)
            .map_err(Self::storage_error)?
            .iter()
            .any(|record| record.metadata.contains_key("external_cgroup_stale"));
        let diagnostic_id = self
            .storage
            .next_diagnostic_id_seed()
            .map_err(Self::storage_error)?;
        let transaction = self.storage.begin().map_err(Self::storage_error)?;
        let result = (|| {
            self.storage
                .mark_cgroup_v2_binding_stale(trace_id, reason, now)?;
            self.storage
                .update_trace_health(trace_id, TraceHealth::Degraded)?;
            if !already_diagnosed {
                self.storage.append_diagnostic(
                    DiagnosticRecord::new(
                        model_core::ids::DiagnosticId::new(diagnostic_id),
                        Some(trace_id),
                        DiagnosticKind::RuntimeFailure,
                        DiagnosticSeverity::Warning,
                        now,
                        "external cgroup accounting lost during recovery; sampling suspended",
                    )
                    .with_metadata("external_cgroup_stale", reason.as_storage_str()),
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => transaction.commit().map_err(Self::storage_error),
            Err(error) => {
                transaction.rollback().map_err(Self::storage_error)?;
                Err(Self::storage_error(error))
            }
        }
    }
}
