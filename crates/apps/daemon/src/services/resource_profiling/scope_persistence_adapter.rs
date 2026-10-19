use super::{
    external_scope_storage::ExternalScopeStorage, managed_scope_storage::ManagedScopeStorage,
};
use model_core::ids::TraceId;
use resource_profiling::model::external_cgroup::ExternalBindingStaleReason;
use resource_profiling::model::external_cgroup::ExternalCgroupBinding;
use resource_profiling::{
    ExternalRecoveryRecord, ExternalScopePersistence, ManagedRecoveryRecord,
    ManagedScopePersistence, ProfilingError,
};
use std::time::SystemTime;
use storage_core::StorageBackend;
pub(super) struct ScopePersistenceAdapter<'a> {
    pub storage: &'a mut dyn StorageBackend,
}
impl ManagedScopePersistence for ScopePersistenceAdapter<'_> {
    fn prepare_recovery(&mut self) -> Result<Vec<ManagedRecoveryRecord>, ProfilingError> {
        ManagedScopeStorage {
            storage: self.storage,
        }
        .prepare_recovery()
    }
    fn finalize_lost_scope(&mut self, id: TraceId, reason: &str) -> Result<(), ProfilingError> {
        ManagedScopeStorage {
            storage: self.storage,
        }
        .finalize_lost_scope(id, reason)
    }
}
impl ExternalScopePersistence for ScopePersistenceAdapter<'_> {
    fn prepare_external_recovery(&mut self) -> Result<Vec<ExternalRecoveryRecord>, ProfilingError> {
        ExternalScopeStorage {
            storage: self.storage,
        }
        .prepare_external_recovery()
    }
    fn record_success(&mut self, id: TraceId, at: SystemTime) -> Result<(), ProfilingError> {
        ExternalScopeStorage {
            storage: self.storage,
        }
        .record_success(id, at)
    }
    fn record_failure(&mut self, id: TraceId, at: SystemTime) -> Result<u32, ProfilingError> {
        ExternalScopeStorage {
            storage: self.storage,
        }
        .record_failure(id, at)
    }
    fn mark_stale(
        &mut self,
        id: TraceId,
        reason: ExternalBindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        ExternalScopeStorage {
            storage: self.storage,
        }
        .mark_stale(id, reason, at)
    }
    fn mark_recovery_stale(
        &mut self,
        id: TraceId,
        reason: ExternalBindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        ExternalScopeStorage {
            storage: self.storage,
        }
        .mark_recovery_stale(id, reason, at)
    }
    fn binding(&self, id: TraceId) -> Result<Option<ExternalCgroupBinding>, ProfilingError> {
        self.storage
            .get_external_cgroup_binding(id)
            .map_err(|e| ProfilingError::new(e.stage, e.message))
    }
    fn complete_recovered_trace(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        ExternalScopeStorage {
            storage: self.storage,
        }
        .complete_recovered_trace(id, at)
    }
}
