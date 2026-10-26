use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct L2WebSocketDocument {
    pub enabled: bool,
    pub message_summary: bool,
    pub max_active_connections: u32,
    pub max_frame_buffer_bytes: u64,
    pub max_message_bytes: u64,
    pub max_decoded_message_bytes: u64,
    pub max_evidence_ranges: u32,
    pub compact_threshold_bytes: u64,
}
impl Default for L2WebSocketDocument {
    fn default() -> Self {
        Self::from_config(L2WebSocketRetention::default())
    }
}
impl L2WebSocketDocument {
    pub(super) fn from_config(config: L2WebSocketRetention) -> Self {
        Self {
            enabled: config.enabled,
            message_summary: config.message_summary,
            max_active_connections: config.max_active_connections,
            max_frame_buffer_bytes: config.max_frame_buffer_bytes,
            max_message_bytes: config.max_message_bytes,
            max_decoded_message_bytes: config.max_decoded_message_bytes,
            max_evidence_ranges: config.max_evidence_ranges,
            compact_threshold_bytes: config.compact_threshold_bytes,
        }
    }
    pub(super) fn to_config(&self) -> Result<L2WebSocketRetention, String> {
        Ok(L2WebSocketRetention {
            enabled: self.enabled,
            message_summary: self.message_summary,
            max_active_connections: require_positive_u32(
                "semantic_retention.l2_websocket.max_active_connections",
                self.max_active_connections,
            )?,
            max_frame_buffer_bytes: require_positive_u64(
                "semantic_retention.l2_websocket.max_frame_buffer_bytes",
                self.max_frame_buffer_bytes,
            )?,
            max_message_bytes: require_positive_u64(
                "semantic_retention.l2_websocket.max_message_bytes",
                self.max_message_bytes,
            )?,
            max_decoded_message_bytes: require_positive_u64(
                "semantic_retention.l2_websocket.max_decoded_message_bytes",
                self.max_decoded_message_bytes,
            )?,
            max_evidence_ranges: require_positive_u32(
                "semantic_retention.l2_websocket.max_evidence_ranges",
                self.max_evidence_ranges,
            )?,
            compact_threshold_bytes: require_positive_u64(
                "semantic_retention.l2_websocket.compact_threshold_bytes",
                self.compact_threshold_bytes,
            )?,
        })
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct LlmWebSocketDocument {
    pub max_lanes_per_connection: u32,
    pub max_pending_requests_per_connection: u32,
}
impl Default for LlmWebSocketDocument {
    fn default() -> Self {
        Self::from_config(LlmWebSocketConfig::default())
    }
}
impl LlmWebSocketDocument {
    pub(super) fn from_config(config: LlmWebSocketConfig) -> Self {
        Self {
            max_lanes_per_connection: config.max_lanes_per_connection,
            max_pending_requests_per_connection: config.max_pending_requests_per_connection,
        }
    }
    pub(super) fn to_config(&self) -> Result<LlmWebSocketConfig, String> {
        Ok(LlmWebSocketConfig {
            max_lanes_per_connection: require_positive_u32(
                "semantic_retention.l0_llm_call.websocket.max_lanes_per_connection",
                self.max_lanes_per_connection,
            )?,
            max_pending_requests_per_connection: require_positive_u32(
                "semantic_retention.l0_llm_call.websocket.max_pending_requests_per_connection",
                self.max_pending_requests_per_connection,
            )?,
        })
    }
}
