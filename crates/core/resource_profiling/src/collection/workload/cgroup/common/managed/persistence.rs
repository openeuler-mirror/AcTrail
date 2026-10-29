use crate::ProfilingError;
use crate::model::resource_scope::TraceResourceScope;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

#[derive(Clone)]
pub struct ManagedRecoveryRecord {
    pub scope: TraceResourceScope,
    pub root: ProcessIdentity,
}

/// Transactions and trace/event mutations belong to the persistence adapter.
pub trait ManagedScopePersistence {
    fn prepare_recovery(&mut self) -> Result<Vec<ManagedRecoveryRecord>, ProfilingError>;
    fn finalize_lost_scope(&mut self, trace: TraceId, reason: &str) -> Result<(), ProfilingError>;
}
