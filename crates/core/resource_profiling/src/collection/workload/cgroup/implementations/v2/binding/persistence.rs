use crate::ProfilingError;
use crate::model::cgroup_v2_binding::CgroupV2Binding;
use crate::model::cgroup_v2_binding::CgroupV2BindingStaleReason;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::time::SystemTime;
pub struct CgroupV2BindingRecovery {
    pub binding: CgroupV2Binding,
    pub root: ProcessIdentity,
    pub terminal: bool,
}
/// Durable state of [`CgroupV2Binding`] records.
pub trait CgroupV2BindingPersistence {
    fn prepare_v2_binding_recovery(
        &mut self,
    ) -> Result<Vec<CgroupV2BindingRecovery>, ProfilingError>;
    fn record_v2_binding_success(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError>;
    fn record_v2_binding_failure(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<u32, ProfilingError>;
    fn mark_v2_binding_stale(
        &mut self,
        id: TraceId,
        reason: CgroupV2BindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError>;
    fn mark_v2_binding_recovery_stale(
        &mut self,
        id: TraceId,
        reason: CgroupV2BindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError>;
    fn v2_binding(&self, id: TraceId) -> Result<Option<CgroupV2Binding>, ProfilingError>;
    fn complete_v2_binding_recovered_trace(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError>;
}
