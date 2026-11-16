use crate::StorageError;
use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v1_binding::CgroupV1Binding;
use std::time::SystemTime;
pub trait CgroupV1BindingStore {
    fn create_cgroup_v1_binding(&mut self, binding: &CgroupV1Binding) -> Result<(), StorageError>;
    fn get_cgroup_v1_binding(&self, id: TraceId) -> Result<Option<CgroupV1Binding>, StorageError>;
    fn list_live_cgroup_v1_bindings(&self) -> Result<Vec<CgroupV1Binding>, StorageError>;
    fn mark_cgroup_v1_binding_stale(
        &mut self,
        id: TraceId,
        reason: &str,
    ) -> Result<(), StorageError>;
    fn close_cgroup_v1_binding(
        &mut self,
        trace_id: TraceId,
        at: SystemTime,
    ) -> Result<(), StorageError>;
}
