use crate::NoOpStorage;
use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v2_binding::{CgroupV2Binding, CgroupV2BindingStaleReason};
use std::time::SystemTime;
use storage_core::{CgroupV2BindingStore, StorageError};
impl CgroupV2BindingStore for NoOpStorage {
    fn create_cgroup_v2_binding(&mut self, binding: CgroupV2Binding) -> Result<(), StorageError> {
        self.resources.borrow_mut().create_binding(binding)
    }

    fn get_cgroup_v2_binding(&self, id: TraceId) -> Result<Option<CgroupV2Binding>, StorageError> {
        Ok(self.resources.borrow().binding(id))
    }

    fn list_live_cgroup_v2_bindings(&self) -> Result<Vec<CgroupV2Binding>, StorageError> {
        Ok(self.resources.borrow().live_bindings())
    }

    fn record_cgroup_v2_binding_success(
        &mut self,
        id: TraceId,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        self.resources.borrow_mut().success(id, time)
    }

    fn record_cgroup_v2_binding_failure(
        &mut self,
        id: TraceId,
        time: SystemTime,
    ) -> Result<u32, StorageError> {
        self.resources.borrow_mut().failure(id, time)
    }

    fn mark_cgroup_v2_binding_stale(
        &mut self,
        id: TraceId,
        reason: CgroupV2BindingStaleReason,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        self.resources.borrow_mut().stale(id, reason, time)
    }

    fn discard_orphan_cgroup_v2_binding(&mut self, id: TraceId) -> Result<(), StorageError> {
        self.resources.borrow_mut().discard_orphan(id);
        Ok(())
    }

    fn close_cgroup_v2_binding(
        &mut self,
        id: TraceId,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        self.resources.borrow_mut().close(id, time)
    }
}
