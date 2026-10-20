//! Stateless retention policy over facts supplied by the protocol owner.

use config_core::daemon::{Http2DataContentRetention, SemanticRetentionConfig};
use model_core::payload::PayloadSegment;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::services) enum PayloadBodyRetention {
    Full,
    SummaryOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::services) struct PayloadBodyRetentionDecision {
    pub(in crate::services) mode: PayloadBodyRetention,
    pub(in crate::services) semantic_layer: PayloadSemanticLayer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::services) enum PayloadSemanticLayer {
    None,
    Http,
    Llm,
}
impl PayloadSemanticLayer {
    pub(in crate::services) fn consumed_by_higher_layer(self) -> bool {
        matches!(self, Self::Http | Self::Llm)
    }
    pub(in crate::services) fn consumed_by_llm(self) -> bool {
        self == Self::Llm
    }
}

pub(in crate::services) struct PayloadBodyRetentionGate {
    semantic_retention: SemanticRetentionConfig,
}
impl PayloadBodyRetentionGate {
    pub(in crate::services) fn new(semantic_retention: SemanticRetentionConfig) -> Self {
        Self { semantic_retention }
    }

    pub(in crate::services) fn discontinuity_decision() -> PayloadBodyRetentionDecision {
        PayloadBodyRetentionDecision {
            mode: PayloadBodyRetention::Full,
            semantic_layer: PayloadSemanticLayer::None,
        }
    }

    pub(in crate::services) fn decide(
        &self,
        segment: &PayloadSegment,
        consumed_by_http: bool,
        consumed_by_llm: bool,
    ) -> PayloadBodyRetentionDecision {
        if segment
            .protocol_hint
            .as_deref()
            .is_some_and(|hint| hint.starts_with("tls-summary;"))
        {
            return PayloadBodyRetentionDecision {
                mode: PayloadBodyRetention::SummaryOnly,
                semantic_layer: PayloadSemanticLayer::Http,
            };
        }
        let semantic_layer = if consumed_by_llm {
            PayloadSemanticLayer::Llm
        } else if consumed_by_http {
            PayloadSemanticLayer::Http
        } else {
            PayloadSemanticLayer::None
        };
        // Unknown and mixed segments retain their original data. No connection-wide
        // classification survives a transaction or changes an earlier observation.
        let mode = if semantic_layer == PayloadSemanticLayer::None
            || self
                .semantic_retention
                .http_body_content_needed(consumed_by_llm)
            || self.semantic_retention.http2_frame_summary_enabled()
            || self.semantic_retention.http2_data_content() != Http2DataContentRetention::None
        {
            PayloadBodyRetention::Full
        } else {
            PayloadBodyRetention::SummaryOnly
        };
        PayloadBodyRetentionDecision {
            mode,
            semantic_layer,
        }
    }
}
