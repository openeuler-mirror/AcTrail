//! Agent-specific launch integration and its independent configuration.
mod config;
mod execution;
mod launch;
mod opencode;

pub use config::AgentHostConfig;
pub use execution::{ExecutionKey, ExecutionState, ExecutionStates};
pub use launch::AgentLaunchIntegration;
