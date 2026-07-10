//! Runtime projection from low-level facts into semantic actions.

pub mod lineage_projection;
pub mod live;
mod payload_projection;

pub use lineage_projection::derive_lineage_links;
pub use live::LiveSemanticActionRuntime;
pub use payload_projection::llm::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRegistry,
    LlmCodecRequest, LlmCodecSseEvent,
};
