use crate::ProfilingError;
use crate::model::external_cgroup::ExternalBindingStaleReason;
use crate::model::external_cgroup::ExternalCgroupBinding;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::time::SystemTime;
pub struct ExternalRecoveryRecord {
    pub binding: ExternalCgroupBinding,
    pub root: ProcessIdentity,
    pub terminal: bool,
}
pub trait ExternalScopePersistence {
    fn prepare_external_recovery(&mut self) -> Result<Vec<ExternalRecoveryRecord>, ProfilingError>;
    fn record_success(&mut self, id: TraceId, at: SystemTime) -> Result<(), ProfilingError>;
    fn record_failure(&mut self, id: TraceId, at: SystemTime) -> Result<u32, ProfilingError>;
    fn mark_stale(
        &mut self,
        id: TraceId,
        reason: ExternalBindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError>;
    fn mark_recovery_stale(
        &mut self,
        id: TraceId,
        reason: ExternalBindingStaleReason,
        at: SystemTime,
    ) -> Result<(), ProfilingError>;
    fn binding(&self, id: TraceId) -> Result<Option<ExternalCgroupBinding>, ProfilingError>;
    fn complete_recovered_trace(
        &mut self,
        id: TraceId,
        at: SystemTime,
    ) -> Result<(), ProfilingError>;
}
