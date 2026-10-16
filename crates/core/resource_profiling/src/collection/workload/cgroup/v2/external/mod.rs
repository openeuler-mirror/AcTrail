mod config;
pub use config::{ExternalScopeConfig, ExternalScopePolicy};
mod binding;
pub use binding::{ContainerCgroupResolver, ExternalScopeBinding};
mod external_scope_manager;
mod external_scope_persistence;
pub(crate) use external_scope_manager::{
    ExternalAdmission, ExternalFinalDraft, ExternalRead, ExternalScopeManager,
};
pub use external_scope_persistence::{ExternalRecoveryRecord, ExternalScopePersistence};
