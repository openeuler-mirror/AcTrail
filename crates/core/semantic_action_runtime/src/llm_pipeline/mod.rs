//! LLM streaming-to-action pipeline.
//!
//! The public surface is intentionally limited to the facade. Transport,
//! streaming, provider, and projection components communicate through their
//! re-exported boundary types rather than importing sibling internals.

mod assembly;
pub(crate) mod config;
mod facade;
mod projection;
mod provider;
mod stream;
mod transport;

pub(crate) use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};

pub(super) use facade::{
    ActionBatch, LlmActionPipeline, LlmHttpRequestLink, LlmHttpResponseLink, PipelineEvent,
};
pub(crate) use projection::{ProjectedLlmToolResult, canonical_llm_json};
pub use provider::codec::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRegistry,
    LlmCodecRequest, LlmCodecSseEvent,
};
