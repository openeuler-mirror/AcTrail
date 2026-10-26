mod config;
pub use config::ManagedScopeConfig;
mod managed_scope_manager;
mod managed_scope_persistence;
mod scope_recovery;
pub(crate) use managed_scope_manager::{CgroupAdmission, ManagedScopeManager};
pub use managed_scope_persistence::{ManagedRecoveryRecord, ManagedScopePersistence};
