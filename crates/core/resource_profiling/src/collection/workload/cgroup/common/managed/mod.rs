//! Lifecycle of AcTrail-created trace scopes: admission, sampling, finalization
//! and restart recovery. Written once; each cgroup version supplies the
//! filesystem operations through [`ManagedHierarchy`].

mod completion;
mod hierarchy;
mod manager;
mod payload;
mod persistence;
mod recovery;
mod runtime;
pub use completion::{ResourceFinalizationDraft, ResourceFinalizationPoll};
pub(crate) use hierarchy::ManagedHierarchy;
pub(crate) use manager::{ManagedScopeAdmission, ManagedScopeManager};
pub(crate) use payload::apply_cgroup_counters;
pub use persistence::{ManagedRecoveryRecord, ManagedScopePersistence};
pub(crate) use runtime::ManagedScopeRuntime;
