//! Attach-time orchestration skeleton for existing processes.

pub mod bootstrap;
pub mod coverage_guard;
pub mod identity_merge;
pub mod snapshot_merge;

pub use bootstrap::{AttachRequest, BootstrapCoordinator, BootstrapError, BootstrapResult};
