mod bindings;

use crate::SqliteStorage;
use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v2_binding::{CgroupV2Binding, CgroupV2BindingStaleReason};
use storage_core::{CgroupV2BindingStore, StorageError};

impl CgroupV2BindingStore for SqliteStorage {
    fn create_cgroup_v2_binding(&mut self, binding: CgroupV2Binding) -> Result<(), StorageError> {
        SqliteStorage::create_cgroup_v2_binding(self, &binding)
            .map_err(|error| StorageError::new("create_external_cgroup_binding", error.to_string()))
    }

    fn get_cgroup_v2_binding(
        &self,
        trace_id: TraceId,
    ) -> Result<Option<CgroupV2Binding>, StorageError> {
        SqliteStorage::get_cgroup_v2_binding(self, trace_id)
            .map_err(|error| StorageError::new("get_external_cgroup_binding", error.to_string()))
    }

    fn list_live_cgroup_v2_bindings(&self) -> Result<Vec<CgroupV2Binding>, StorageError> {
        SqliteStorage::list_live_cgroup_v2_bindings(self).map_err(|error| {
            StorageError::new("list_live_external_cgroup_bindings", error.to_string())
        })
    }

    fn record_cgroup_v2_binding_success(
        &mut self,
        trace_id: TraceId,
        observed_at: std::time::SystemTime,
    ) -> Result<(), StorageError> {
        SqliteStorage::record_cgroup_v2_binding_success(self, trace_id, observed_at)
            .map_err(|error| StorageError::new("record_external_cgroup_success", error.to_string()))
    }

    fn record_cgroup_v2_binding_failure(
        &mut self,
        trace_id: TraceId,
        observed_at: std::time::SystemTime,
    ) -> Result<u32, StorageError> {
        SqliteStorage::record_cgroup_v2_binding_failure(self, trace_id, observed_at)
            .map_err(|error| StorageError::new("record_external_cgroup_failure", error.to_string()))
    }

    fn mark_cgroup_v2_binding_stale(
        &mut self,
        trace_id: TraceId,
        reason: CgroupV2BindingStaleReason,
        updated_at: std::time::SystemTime,
    ) -> Result<(), StorageError> {
        SqliteStorage::mark_cgroup_v2_binding_stale(self, trace_id, reason, updated_at)
            .map_err(|error| StorageError::new("mark_external_cgroup_stale", error.to_string()))
    }

    fn discard_orphan_cgroup_v2_binding(&mut self, trace_id: TraceId) -> Result<(), StorageError> {
        self.connection()
            .borrow_mut()
            .execute(
                "DELETE FROM trace_external_cgroup_v2_bindings
             WHERE trace_id=?1 AND lifecycle_state IN ('active','stale')
             AND NOT EXISTS (SELECT 1 FROM traces WHERE trace_id=?1)",
                rusqlite::params![trace_id.get()],
            )
            .map(|_| ())
            .map_err(|error| {
                StorageError::new("discard_orphan_external_binding", error.to_string())
            })
    }

    fn close_cgroup_v2_binding(
        &mut self,
        trace_id: TraceId,
        closed_at: std::time::SystemTime,
    ) -> Result<(), StorageError> {
        SqliteStorage::close_cgroup_v2_binding(self, trace_id, closed_at)
            .map_err(|e| StorageError::new("close_external_cgroup_binding", e.to_string()))
    }
}
