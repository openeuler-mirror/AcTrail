//! Trace lifecycle and membership orchestration skeleton.

pub mod commands;
pub mod membership;
pub mod registry;
pub mod sensor_plan;
pub mod state_machine;

pub use registry::{TraceOwnerPrincipal, TraceRuntime};
