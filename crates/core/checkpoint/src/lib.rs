//! Checkpoint coordination: scope planning, assembling the backend of each Trace, per-Trace
//! scheduling, the record model and the ports it relies on.

mod backends;
pub mod config;
pub mod coordinator;
pub mod factory;
pub mod model;
pub mod ports;
pub mod scope;

pub use config::BackendEntry;
pub use coordinator::{
    BoundaryAction, CheckpointCoordinator, CompletedJob, Completion, CompletionSink,
    CoordinatorError, Hold, LaunchRequest,
};
pub use factory::CheckpointFactory;
