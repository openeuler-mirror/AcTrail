//! Observation-time semantic action projection.

mod actions;
mod agent;
mod command;
mod file;
mod http_exchange;
mod links;
mod mcp;
mod process_parent;
mod runtime;
mod tool;

pub use mcp::LiveMcpStdioDiagnostic;
pub use runtime::LiveSemanticActionObservation;
pub use runtime::{LiveMcpStdioMetrics, LiveSemanticActionOutput, LiveSemanticActionRuntime};

pub(crate) use actions::{
    action_for_live_state, append_missing_evidence, llm_call_action_id_from_request_action_id,
};
pub(crate) use http_exchange::{HttpResponseMatch, MatchedHttpRequest};
