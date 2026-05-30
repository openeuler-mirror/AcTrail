//! Runtime projection from low-level facts into semantic actions.

pub mod live;
pub mod snapshot;

pub use live::LiveSemanticActionRuntime;
pub use snapshot::project_snapshot_actions;
