//! Observation-time semantic action projection.

mod actions;
mod agent;
mod command;
mod file;
mod links;
mod llm;
mod mcp;
mod process_parent;
mod runtime;

pub use mcp::LiveMcpStdioDiagnostic;
pub use runtime::LiveSemanticActionObservation;
pub use runtime::{LiveMcpStdioMetrics, LiveSemanticActionOutput, LiveSemanticActionRuntime};
