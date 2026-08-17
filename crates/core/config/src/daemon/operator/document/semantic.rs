use super::*;
use crate::daemon::agent::DEFAULT_LLM_WEBSOCKET_MAX_CONNECTIONS_PER_PROCESS;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SemanticRetentionDocument {
    pub content_owner: String,
    pub l0_llm_call: L0LlmCallDocument,
    pub l0_mcp_call: L0McpCallDocument,
    pub l1_sse: L1SseDocument,
    pub l2_http: L2HttpDocument,
    pub l3_http2_frame: L3Http2FrameDocument,
    pub l4_payload: L4PayloadDocument,
}

impl Default for SemanticRetentionDocument {
    fn default() -> Self {
        Self {
            content_owner: "highest_consumed".to_string(),
            l0_llm_call: L0LlmCallDocument::default(),
            l0_mcp_call: L0McpCallDocument::default(),
            l1_sse: L1SseDocument::default(),
            l2_http: L2HttpDocument::default(),
            l3_http2_frame: L3Http2FrameDocument::default(),
            l4_payload: L4PayloadDocument::default(),
        }
    }
}

impl SemanticRetentionDocument {
    pub(super) fn from_config(config: &SemanticRetentionConfig) -> Self {
        Self {
            content_owner: semantic_content_owner_as_str(config.content_owner).to_string(),
            l0_llm_call: L0LlmCallDocument {
                enabled: config.l0_llm_call.enabled,
                request_content: llm_request_content_retention_as_str(
                    config.l0_llm_call.request_content,
                )
                .to_string(),
                response_content: llm_response_content_retention_as_str(
                    config.l0_llm_call.response_content,
                )
                .to_string(),
                tool_calls: llm_tool_call_retention_as_str(config.l0_llm_call.tool_calls)
                    .to_string(),
                usage: llm_usage_retention_as_str(config.l0_llm_call.usage).to_string(),
                retain_assembled_payload: config.l0_llm_call.retain_assembled_payload,
                websocket_max_connections_per_process: config
                    .l0_llm_call
                    .websocket_max_connections_per_process,
                trajectory: LlmTrajectoryDocument::from_config(&config.l0_llm_call.trajectory),
            },
            l0_mcp_call: L0McpCallDocument {
                request_content: mcp_jsonrpc_content_retention_as_str(
                    config.l0_mcp_call.request_content,
                )
                .to_string(),
                response_content: mcp_jsonrpc_content_retention_as_str(
                    config.l0_mcp_call.response_content,
                )
                .to_string(),
            },
            l1_sse: L1SseDocument {
                enabled: config.l1_sse.enabled,
                stream_summary: config.l1_sse.stream_summary,
                event_content: sse_event_content_retention_as_str(config.l1_sse.event_content)
                    .to_string(),
            },
            l2_http: L2HttpDocument {
                enabled: config.l2_http.enabled,
                message_summary: config.l2_http.message_summary,
                headers: http_headers_retention_as_str(config.l2_http.headers).to_string(),
                body_content: http_body_retention_as_str(config.l2_http.body_content).to_string(),
            },
            l3_http2_frame: L3Http2FrameDocument {
                enabled: config.l3_http2_frame.enabled,
                frame_summary: config.l3_http2_frame.frame_summary,
                data_content: http2_data_content_retention_as_str(
                    config.l3_http2_frame.data_content,
                )
                .to_string(),
            },
            l4_payload: L4PayloadDocument {
                enabled: config.l4_payload.enabled,
                stats: config.l4_payload.stats,
                body_content: payload_body_content_retention_as_str(config.l4_payload.body_content)
                    .to_string(),
            },
        }
    }

    pub(super) fn to_config(&self) -> Result<SemanticRetentionConfig, String> {
        Ok(SemanticRetentionConfig {
            content_owner: parse_value("semantic_retention.content_owner", &self.content_owner)?,
            l0_llm_call: self.l0_llm_call.to_config()?,
            l0_mcp_call: self.l0_mcp_call.to_config()?,
            l1_sse: self.l1_sse.to_config()?,
            l2_http: self.l2_http.to_config()?,
            l3_http2_frame: self.l3_http2_frame.to_config()?,
            l4_payload: self.l4_payload.to_config()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct L0McpCallDocument {
    pub request_content: String,
    pub response_content: String,
}

impl Default for L0McpCallDocument {
    fn default() -> Self {
        Self {
            request_content: "canonical_json".to_string(),
            response_content: "canonical_json".to_string(),
        }
    }
}

impl L0McpCallDocument {
    pub(super) fn to_config(&self) -> Result<L0McpCallRetention, String> {
        Ok(L0McpCallRetention {
            request_content: parse_value(
                "semantic_retention.l0_mcp_call.request_content",
                &self.request_content,
            )?,
            response_content: parse_value(
                "semantic_retention.l0_mcp_call.response_content",
                &self.response_content,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct L0LlmCallDocument {
    pub enabled: bool,
    pub request_content: String,
    pub response_content: String,
    pub tool_calls: String,
    pub usage: String,
    pub retain_assembled_payload: bool,
    pub websocket_max_connections_per_process: u32,
    pub trajectory: LlmTrajectoryDocument,
}

impl Default for L0LlmCallDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            request_content: "canonical_blocks".to_string(),
            response_content: "assembled_provider".to_string(),
            tool_calls: "assembled_json".to_string(),
            usage: "summary".to_string(),
            retain_assembled_payload: false,
            websocket_max_connections_per_process:
                DEFAULT_LLM_WEBSOCKET_MAX_CONNECTIONS_PER_PROCESS,
            trajectory: LlmTrajectoryDocument::default(),
        }
    }
}

impl L0LlmCallDocument {
    pub(super) fn to_config(&self) -> Result<L0LlmCallRetention, String> {
        Ok(L0LlmCallRetention {
            enabled: self.enabled,
            request_content: parse_value(
                "semantic_retention.l0_llm_call.request_content",
                &self.request_content,
            )?,
            response_content: parse_value(
                "semantic_retention.l0_llm_call.response_content",
                &self.response_content,
            )?,
            tool_calls: parse_value(
                "semantic_retention.l0_llm_call.tool_calls",
                &self.tool_calls,
            )?,
            usage: parse_value("semantic_retention.l0_llm_call.usage", &self.usage)?,
            retain_assembled_payload: self.retain_assembled_payload,
            websocket_max_connections_per_process: require_positive_u32(
                "semantic_retention.l0_llm_call.websocket_max_connections_per_process",
                self.websocket_max_connections_per_process,
            )?,
            trajectory: self.trajectory.to_config()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LlmTrajectoryDocument {
    pub enabled: bool,
    pub max_active_trajectories_per_scope: u32,
    pub max_candidate_nodes_per_trajectory: u32,
    pub max_prefix_nodes_per_scope: u32,
    pub max_history_atoms_per_request: u32,
    pub max_blocks_per_atom: u32,
    pub max_structural_bytes_per_atom: u32,
    pub idle_ttl: String,
}

impl Default for LlmTrajectoryDocument {
    fn default() -> Self {
        Self::from_config(&LlmTrajectoryConfig::default())
    }
}

impl LlmTrajectoryDocument {
    fn from_config(config: &LlmTrajectoryConfig) -> Self {
        Self {
            enabled: config.enabled,
            max_active_trajectories_per_scope: config.max_active_trajectories_per_scope,
            max_candidate_nodes_per_trajectory: config.max_candidate_nodes_per_trajectory,
            max_prefix_nodes_per_scope: config.max_prefix_nodes_per_scope,
            max_history_atoms_per_request: config.max_history_atoms_per_request,
            max_blocks_per_atom: config.max_blocks_per_atom,
            max_structural_bytes_per_atom: config.max_structural_bytes_per_atom,
            idle_ttl: duration_as_string(config.idle_ttl),
        }
    }

    fn to_config(&self) -> Result<LlmTrajectoryConfig, String> {
        Ok(LlmTrajectoryConfig {
            enabled: self.enabled,
            max_active_trajectories_per_scope: require_positive_u32(
                "semantic_retention.l0_llm_call.trajectory.max_active_trajectories_per_scope",
                self.max_active_trajectories_per_scope,
            )?,
            max_candidate_nodes_per_trajectory: require_positive_u32(
                "semantic_retention.l0_llm_call.trajectory.max_candidate_nodes_per_trajectory",
                self.max_candidate_nodes_per_trajectory,
            )?,
            max_prefix_nodes_per_scope: require_positive_u32(
                "semantic_retention.l0_llm_call.trajectory.max_prefix_nodes_per_scope",
                self.max_prefix_nodes_per_scope,
            )?,
            max_history_atoms_per_request: require_positive_u32(
                "semantic_retention.l0_llm_call.trajectory.max_history_atoms_per_request",
                self.max_history_atoms_per_request,
            )?,
            max_blocks_per_atom: require_positive_u32(
                "semantic_retention.l0_llm_call.trajectory.max_blocks_per_atom",
                self.max_blocks_per_atom,
            )?,
            max_structural_bytes_per_atom: require_positive_u32(
                "semantic_retention.l0_llm_call.trajectory.max_structural_bytes_per_atom",
                self.max_structural_bytes_per_atom,
            )?,
            idle_ttl: parse_required_duration(
                "semantic_retention.l0_llm_call.trajectory.idle_ttl",
                &self.idle_ttl,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct L1SseDocument {
    pub enabled: bool,
    pub stream_summary: bool,
    pub event_content: String,
}

impl Default for L1SseDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            stream_summary: true,
            event_content: "none".to_string(),
        }
    }
}

impl L1SseDocument {
    pub(super) fn to_config(&self) -> Result<L1SseRetention, String> {
        Ok(L1SseRetention {
            enabled: self.enabled,
            stream_summary: self.stream_summary,
            event_content: parse_value(
                "semantic_retention.l1_sse.event_content",
                &self.event_content,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct L2HttpDocument {
    pub enabled: bool,
    pub message_summary: bool,
    pub headers: String,
    pub body_content: String,
}

impl Default for L2HttpDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            message_summary: true,
            headers: "metadata".to_string(),
            body_content: "text".to_string(),
        }
    }
}

impl L2HttpDocument {
    pub(super) fn to_config(&self) -> Result<L2HttpRetention, String> {
        Ok(L2HttpRetention {
            enabled: self.enabled,
            message_summary: self.message_summary,
            headers: parse_value("semantic_retention.l2_http.headers", &self.headers)?,
            body_content: parse_value(
                "semantic_retention.l2_http.body_content",
                &self.body_content,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct L3Http2FrameDocument {
    pub enabled: bool,
    pub frame_summary: bool,
    pub data_content: String,
}

impl Default for L3Http2FrameDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            frame_summary: true,
            data_content: "none".to_string(),
        }
    }
}

impl L3Http2FrameDocument {
    pub(super) fn to_config(&self) -> Result<L3Http2FrameRetention, String> {
        Ok(L3Http2FrameRetention {
            enabled: self.enabled,
            frame_summary: self.frame_summary,
            data_content: parse_value(
                "semantic_retention.l3_http2_frame.data_content",
                &self.data_content,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct L4PayloadDocument {
    pub enabled: bool,
    pub stats: bool,
    pub body_content: String,
}

impl Default for L4PayloadDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            stats: true,
            body_content: "none".to_string(),
        }
    }
}

impl L4PayloadDocument {
    pub(super) fn to_config(&self) -> Result<L4PayloadRetention, String> {
        Ok(L4PayloadRetention {
            enabled: self.enabled,
            stats: self.stats,
            body_content: parse_value(
                "semantic_retention.l4_payload.body_content",
                &self.body_content,
            )?,
        })
    }
}
