use crate::StorageError;
use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v2_binding::{CgroupV2Binding, CgroupV2BindingStaleReason};
use std::time::SystemTime;
pub trait CgroupV2BindingStore {
    fn create_cgroup_v2_binding(&mut self, binding: CgroupV2Binding) -> Result<(), StorageError>;
    fn get_cgroup_v2_binding(
        &self,
        trace_id: TraceId,
    ) -> Result<Option<CgroupV2Binding>, StorageError>;
    fn list_live_cgroup_v2_bindings(&self) -> Result<Vec<CgroupV2Binding>, StorageError>;
    fn record_cgroup_v2_binding_success(
        &mut self,
        trace_id: TraceId,
        observed_at: SystemTime,
    ) -> Result<(), StorageError>;
    fn record_cgroup_v2_binding_failure(
        &mut self,
        trace_id: TraceId,
        observed_at: SystemTime,
    ) -> Result<u32, StorageError>;
    fn mark_cgroup_v2_binding_stale(
        &mut self,
        trace_id: TraceId,
        reason: CgroupV2BindingStaleReason,
        updated_at: SystemTime,
    ) -> Result<(), StorageError>;
    /// Discard a legacy interrupted admission only if its trace does not exist.
    fn discard_orphan_cgroup_v2_binding(&mut self, trace_id: TraceId) -> Result<(), StorageError>;

    fn close_cgroup_v2_binding(
        &mut self,
        trace_id: TraceId,
        closed_at: SystemTime,
    ) -> Result<(), StorageError>;
}
