use super::super::scope_persistence_adapter::ScopePersistenceAdapter;
use model_core::{
    ids::TraceId,
    trace::{TraceHealth, TraceLifecycleState},
};
use resource_profiling::{
    CgroupPersistence, CgroupV1Persistence, CgroupV1RecoveryRecord, ProfilingError,
};
use std::time::SystemTime;
struct V1StorageError;
impl V1StorageError {
    fn map(error: storage_core::StorageError) -> ProfilingError {
        ProfilingError::new(error.stage, error.message)
    }
}
impl CgroupV1Persistence for ScopePersistenceAdapter<'_> {
    fn recover_v1(&mut self) -> Result<Vec<CgroupV1RecoveryRecord>, ProfilingError> {
        self.storage
            .live_cgroup_v1_bindings()
            .map_err(V1StorageError::map)?
            .into_iter()
            .map(|binding| {
                let trace = self
                    .storage
                    .get_trace(binding.trace_id)
                    .map_err(V1StorageError::map)?
                    .ok_or_else(|| {
                        ProfilingError::new("cgroup_v1_recovery", "binding has no trace")
                    })?;
                Ok(CgroupV1RecoveryRecord {
                    binding,
                    root: trace.root_process_identity,
                    terminal: trace.lifecycle_state.is_terminal(),
                })
            })
            .collect()
    }
    fn stale_v1(&mut self, id: TraceId, reason: &str) -> Result<(), ProfilingError> {
        let transaction = self.storage.begin().map_err(V1StorageError::map)?;
        let result = self
            .storage
            .mark_cgroup_v1_stale(id, reason)
            .and_then(|_| self.storage.update_trace_health(id, TraceHealth::Degraded));
        match result {
            Ok(()) => transaction.commit().map_err(V1StorageError::map),
            Err(error) => {
                transaction.rollback().map_err(V1StorageError::map)?;
                Err(V1StorageError::map(error))
            }
        }
    }
    fn complete_v1_trace(&mut self, id: TraceId, at: SystemTime) -> Result<(), ProfilingError> {
        let mut trace = self
            .storage
            .get_trace(id)
            .map_err(V1StorageError::map)?
            .ok_or_else(|| ProfilingError::new("cgroup_v1_recovery", "trace missing"))?;
        if !trace.lifecycle_state.is_terminal() {
            trace.lifecycle_state = TraceLifecycleState::Completed;
            trace.timings.completed_at = Some(at);
            self.storage
                .create_trace(trace)
                .map_err(V1StorageError::map)?;
        }
        Ok(())
    }
}
impl CgroupPersistence for ScopePersistenceAdapter<'_> {
    fn validate_recovery_versions(&mut self, v1: bool, v2: bool) -> Result<(), ProfilingError> {
        let live_v1 = !self
            .storage
            .live_cgroup_v1_bindings()
            .map_err(V1StorageError::map)?
            .is_empty();
        let live_v2 = self
            .storage
            .list_resource_scopes()
            .map_err(V1StorageError::map)?
            .iter()
            .any(|s| s.lifecycle_state.is_live())
            || !self
                .storage
                .list_live_external_cgroup_bindings()
                .map_err(V1StorageError::map)?
                .is_empty();
        if live_v1 && !v1 || live_v2 && !v2 {
            return Err(ProfilingError::new(
                "cgroup_recovery_version",
                "selected versions exclude a live persisted binding",
            ));
        }
        Ok(())
    }
}
