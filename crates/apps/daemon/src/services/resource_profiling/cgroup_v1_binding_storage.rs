use model_core::{
    ids::TraceId,
    trace::{TraceHealth, TraceLifecycleState},
};
use resource_profiling::{CgroupV1BindingPersistence, CgroupV1BindingRecovery, ProfilingError};
use std::time::SystemTime;
use storage_core::StorageBackend;
pub(super) struct CgroupV1BindingStorage<'a> {
    pub storage: &'a mut dyn StorageBackend,
}
impl CgroupV1BindingStorage<'_> {
    fn storage_error(error: storage_core::StorageError) -> ProfilingError {
        ProfilingError::new(error.stage, error.message)
    }
}
impl CgroupV1BindingPersistence for CgroupV1BindingStorage<'_> {
    fn prepare_v1_binding_recovery(
        &mut self,
    ) -> Result<Vec<CgroupV1BindingRecovery>, ProfilingError> {
        self.storage
            .list_live_cgroup_v1_bindings()
            .map_err(Self::storage_error)?
            .into_iter()
            .map(|binding| {
                let trace = self
                    .storage
                    .get_trace(binding.trace_id)
                    .map_err(Self::storage_error)?
                    .ok_or_else(|| {
                        ProfilingError::new("cgroup_v1_recovery", "binding has no trace")
                    })?;
                Ok(CgroupV1BindingRecovery {
                    binding,
                    root: trace.root_process_identity,
                    terminal: trace.lifecycle_state.is_terminal(),
                })
            })
            .collect()
    }
    fn mark_v1_binding_stale(&mut self, id: TraceId, reason: &str) -> Result<(), ProfilingError> {
        let transaction = self.storage.begin().map_err(Self::storage_error)?;
        let result = self
            .storage
            .mark_cgroup_v1_binding_stale(id, reason)
            .and_then(|_| self.storage.update_trace_health(id, TraceHealth::Degraded));
        match result {
            Ok(()) => transaction.commit().map_err(Self::storage_error),
            Err(error) => {
                transaction.rollback().map_err(Self::storage_error)?;
                Err(Self::storage_error(error))
            }
        }
    }
    fn complete_v1_binding_recovered_trace(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        let mut trace = self
            .storage
            .get_trace(id)
            .map_err(Self::storage_error)?
            .ok_or_else(|| ProfilingError::new("cgroup_v1_recovery", "trace missing"))?;
        if !trace.lifecycle_state.is_terminal() {
            trace.lifecycle_state = TraceLifecycleState::Completed;
            trace.timings.completed_at = Some(at);
            self.storage
                .create_trace(trace)
                .map_err(Self::storage_error)?;
        }
        Ok(())
    }
}
