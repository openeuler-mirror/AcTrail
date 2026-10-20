//! Semantic projection of already parsed messages with an authoritative source.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadStreamIdentity;
use semantic_action::{SemanticEvidence, attr_keys as attrs};
use serde_json::Value;

use super::super::ProjectionBatch;

mod stream;
pub(in crate::llm_pipeline) use stream::ParsedProviderStream;

pub(in crate::llm_pipeline) struct ParsedMessageContext {
    pub(in crate::llm_pipeline) action_id: String,
    pub(in crate::llm_pipeline) identity: PayloadStreamIdentity,
    pub(in crate::llm_pipeline) start_at: SystemTime,
    pub(in crate::llm_pipeline) last_at: SystemTime,
    pub(in crate::llm_pipeline) evidence: Vec<SemanticEvidence>,
    pub(in crate::llm_pipeline) payload_bytes: usize,
}

impl ParsedMessageContext {
    pub(super) fn attributes(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                attrs::payload::STREAM_KEY.into(),
                self.identity.stream_key.to_string(),
            ),
            (
                attrs::payload::SOURCE_BOUNDARY.into(),
                format!("{:?}", self.identity.source_boundary),
            ),
        ])
    }
}

pub(in crate::llm_pipeline) struct ParsedMessageProjector;

impl ParsedMessageProjector {
    pub(in crate::llm_pipeline) fn request(
        config: &SemanticRetentionConfig,
        context: &ParsedMessageContext,
        value: Value,
    ) -> Option<ProjectionBatch> {
        super::request::project_parsed_request(config, context, value)
    }
}
