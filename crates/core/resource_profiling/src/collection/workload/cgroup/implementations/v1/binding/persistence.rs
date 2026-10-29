use crate::ProfilingError;
use crate::model::cgroup_v1_binding::CgroupV1Binding;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::time::SystemTime;
pub struct CgroupV1BindingRecovery {
    pub binding: CgroupV1Binding,
    pub root: ProcessIdentity,
    pub terminal: bool,
}
/// Durable state of [`CgroupV1Binding`] records.
pub trait CgroupV1BindingPersistence {
    fn prepare_v1_binding_recovery(
        &mut self,
    ) -> Result<Vec<CgroupV1BindingRecovery>, ProfilingError>;
    fn mark_v1_binding_stale(&mut self, id: TraceId, reason: &str) -> Result<(), ProfilingError>;
    fn complete_v1_binding_recovered_trace(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError>;
}
