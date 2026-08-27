//! Runtime projection from low-level facts into semantic actions.

pub mod lineage_projection;
pub mod live;
mod llm_pipeline;

pub use lineage_projection::derive_lineage_links;
pub use live::LiveSemanticActionRuntime;
pub use llm_pipeline::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRegistry,
    LlmCodecRequest, LlmCodecSseEvent,
};
