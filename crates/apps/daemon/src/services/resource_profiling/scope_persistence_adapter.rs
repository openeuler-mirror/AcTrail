use super::{
    cgroup_v1_binding_storage::CgroupV1BindingStorage,
    cgroup_v2_binding_storage::CgroupV2BindingStorage, managed_scope_storage::ManagedScopeStorage,
};
use model_core::ids::TraceId;
use resource_profiling::model::ResourceAccountingMethod;
use resource_profiling::model::cgroup_v2_binding::CgroupV2Binding;
use resource_profiling::model::cgroup_v2_binding::CgroupV2BindingStaleReason;
use resource_profiling::{
    CgroupPersistence, CgroupV1BindingPersistence, CgroupV1BindingRecovery,
    CgroupV2BindingPersistence, CgroupV2BindingRecovery, ManagedRecoveryRecord,
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
impl CgroupV2BindingPersistence for ScopePersistenceAdapter<'_> {
    fn prepare_v2_binding_recovery(
        &mut self,
    ) -> Result<Vec<CgroupV2BindingRecovery>, ProfilingError> {
        CgroupV2BindingStorage {
            storage: self.storage,
        }
        .prepare_v2_binding_recovery()
    }
    fn record_v2_binding_success(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        CgroupV2BindingStorage {
            storage: self.storage,
        }
        .record_v2_binding_success(id, at)
    }
    fn record_v2_binding_failure(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<u32, ProfilingError> {
        CgroupV2BindingStorage {
            storage: self.storage,
        }
        .record_v2_binding_failure(id, at)
    }
    fn mark_v2_binding_stale(
        &mut self,
        id: TraceId,
        reason: CgroupV2BindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        CgroupV2BindingStorage {
            storage: self.storage,
        }
        .mark_v2_binding_stale(id, reason, at)
    }
    fn mark_v2_binding_recovery_stale(
        &mut self,
        id: TraceId,
        reason: CgroupV2BindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        CgroupV2BindingStorage {
            storage: self.storage,
        }
        .mark_v2_binding_recovery_stale(id, reason, at)
    }
    fn v2_binding(&self, id: TraceId) -> Result<Option<CgroupV2Binding>, ProfilingError> {
        self.storage
            .get_cgroup_v2_binding(id)
            .map_err(storage_error)
    }
    fn complete_v2_binding_recovered_trace(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        CgroupV2BindingStorage {
            storage: self.storage,
        }
        .complete_v2_binding_recovered_trace(id, at)
    }
}
impl CgroupV1BindingPersistence for ScopePersistenceAdapter<'_> {
    fn prepare_v1_binding_recovery(
        &mut self,
    ) -> Result<Vec<CgroupV1BindingRecovery>, ProfilingError> {
        CgroupV1BindingStorage {
            storage: self.storage,
        }
        .prepare_v1_binding_recovery()
    }
    fn mark_v1_binding_stale(&mut self, id: TraceId, reason: &str) -> Result<(), ProfilingError> {
        CgroupV1BindingStorage {
            storage: self.storage,
        }
        .mark_v1_binding_stale(id, reason)
    }
    fn complete_v1_binding_recovered_trace(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError> {
        CgroupV1BindingStorage {
            storage: self.storage,
        }
        .complete_v1_binding_recovered_trace(id, at)
    }
}
fn storage_error(error: storage_core::StorageError) -> ProfilingError {
    ProfilingError::new(error.stage, error.message)
}
impl CgroupPersistence for ScopePersistenceAdapter<'_> {
    fn validate_recovery_versions(&mut self, v1: bool, v2: bool) -> Result<(), ProfilingError> {
        let live_scopes = self
            .storage
            .list_resource_scopes()
            .map_err(storage_error)?
            .into_iter()
            .filter(|s| s.lifecycle_state.is_live())
            .map(|s| s.accounting_method)
            .collect::<Vec<_>>();
        let live_v1 = live_scopes.contains(&ResourceAccountingMethod::CgroupV1)
            || !self
                .storage
                .list_live_cgroup_v1_bindings()
                .map_err(storage_error)?
                .is_empty();
        let live_v2 = live_scopes
            .iter()
            .any(|method| *method != ResourceAccountingMethod::CgroupV1)
            || !self
                .storage
                .list_live_cgroup_v2_bindings()
                .map_err(storage_error)?
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
