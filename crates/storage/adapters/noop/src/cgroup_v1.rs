use crate::NoOpStorage;
use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v1_binding::CgroupV1Binding;
use std::time::SystemTime;
use storage_core::{CgroupV1BindingStore, StorageError};
impl CgroupV1BindingStore for NoOpStorage {
    fn create_cgroup_v1_binding(&mut self, b: &CgroupV1Binding) -> Result<(), StorageError> {
        self.resources.borrow_mut().create_v1(b.clone())
    }
    fn cgroup_v1_binding(&self, id: TraceId) -> Result<Option<CgroupV1Binding>, StorageError> {
        Ok(self.resources.borrow().v1(id))
    }
    fn live_cgroup_v1_bindings(&self) -> Result<Vec<CgroupV1Binding>, StorageError> {
        Ok(self.resources.borrow().live_v1())
    }
    fn mark_cgroup_v1_stale(&mut self, id: TraceId, reason: &str) -> Result<(), StorageError> {
        self.resources.borrow_mut().stale_v1(id, reason)
    }

    fn close_cgroup_v1_binding(&mut self, id: TraceId, at: SystemTime) -> Result<(), StorageError> {
        self.resources.borrow_mut().close_v1(id, at)
    }
}
