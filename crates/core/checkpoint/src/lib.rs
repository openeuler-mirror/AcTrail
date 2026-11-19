//! Checkpoint coordination: scope planning, assembling the backend of each Trace, per-Trace
//! scheduling and backend execution.

mod backends;
pub mod config;
pub mod coordinator;
pub mod factory;
pub mod scope;

pub use config::BackendEntry;
pub use coordinator::{
    BoundaryAction, CheckpointCoordinator, CompletedJob, Completion, CompletionSink,
    CoordinatorError, Hold, LaunchRequest,
};
pub use factory::CheckpointFactory;
