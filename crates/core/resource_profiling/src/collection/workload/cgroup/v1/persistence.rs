use crate::ProfilingError;
use crate::model::cgroup_v1_binding::CgroupV1Binding;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::time::SystemTime;
pub struct CgroupV1RecoveryRecord {
    pub binding: CgroupV1Binding,
    pub root: ProcessIdentity,
    pub terminal: bool,
}
pub trait CgroupV1Persistence {
    fn recover_v1(&mut self) -> Result<Vec<CgroupV1RecoveryRecord>, ProfilingError>;
    fn stale_v1(&mut self, id: TraceId, reason: &str) -> Result<(), ProfilingError>;
    fn complete_v1_trace(&mut self, id: TraceId, at: SystemTime) -> Result<(), ProfilingError>;
}
