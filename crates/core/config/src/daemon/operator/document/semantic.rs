use super::*;
use crate::daemon::agent::{
    DEFAULT_LLM_STREAM_CLASSIFIER_SOFT_SNIFF_MAX_BYTES,
    DEFAULT_LLM_WEBSOCKET_MAX_CONNECTIONS_PER_PROCESS,
};

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
                request_body_export: llm_request_body_export_retention_as_str(
                    config.l0_llm_call.request_body_export,
                )
                .to_string(),
                request_body_export_max_bytes: config.l0_llm_call.request_body_export_max_bytes,
                response_content: llm_response_content_retention_as_str(
                    config.l0_llm_call.response_content,
                )
                .to_string(),
                tool_calls: llm_tool_call_retention_as_str(config.l0_llm_call.tool_calls)
                    .to_string(),
                tool_result_content_export: llm_tool_result_content_export_retention_as_str(
                    config.l0_llm_call.tool_result_content_export,
                )
                .to_string(),
                tool_result_content_export_max_bytes: config
                    .l0_llm_call
                    .tool_result_content_export_max_bytes,
                usage: llm_usage_retention_as_str(config.l0_llm_call.usage).to_string(),
                retain_assembled_payload: config.l0_llm_call.retain_assembled_payload,
                websocket_max_connections_per_process: config
                    .l0_llm_call
                    .websocket_max_connections_per_process,
                assembly: LlmAssemblyDocument::from_config(&config.l0_llm_call.assembly),
                stream_classifier: LlmStreamClassifierDocument::from_config(
                    &config.l0_llm_call.stream_classifier,
                ),
                projection_state: LlmProjectionStateDocument::from_config(
                    &config.l0_llm_call.projection_state,
                ),
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
                exchange: HttpExchangeDocument::from_config(&config.l2_http.exchange),
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
    pub request_body_export: String,
    pub request_body_export_max_bytes: u64,
    pub response_content: String,
    pub tool_calls: String,
    pub tool_result_content_export: String,
    pub tool_result_content_export_max_bytes: u64,
    pub usage: String,
    pub retain_assembled_payload: bool,
    pub websocket_max_connections_per_process: u32,
    pub assembly: LlmAssemblyDocument,
    pub stream_classifier: LlmStreamClassifierDocument,
    pub projection_state: LlmProjectionStateDocument,
    pub trajectory: LlmTrajectoryDocument,
}

impl Default for L0LlmCallDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            request_content: "canonical_blocks".to_string(),
            request_body_export: "none".to_string(),
            request_body_export_max_bytes: DEFAULT_LLM_REQUEST_BODY_EXPORT_MAX_BYTES,
            response_content: "assembled_provider".to_string(),
            tool_calls: "assembled_json".to_string(),
            tool_result_content_export: "none".to_string(),
            tool_result_content_export_max_bytes: DEFAULT_LLM_TOOL_RESULT_EXPORT_MAX_BYTES,
            usage: "summary".to_string(),
            retain_assembled_payload: false,
            websocket_max_connections_per_process:
                DEFAULT_LLM_WEBSOCKET_MAX_CONNECTIONS_PER_PROCESS,
            assembly: LlmAssemblyDocument::default(),
            stream_classifier: LlmStreamClassifierDocument::default(),
            projection_state: LlmProjectionStateDocument::default(),
            trajectory: LlmTrajectoryDocument::default(),
        }
    }
}

impl L0LlmCallDocument {
    pub(super) fn to_config(&self) -> Result<L0LlmCallRetention, String> {
        let request_content = parse_value(
            "semantic_retention.l0_llm_call.request_content",
            &self.request_content,
        )?;
        let request_body_export = parse_value(
            "semantic_retention.l0_llm_call.request_body_export",
            &self.request_body_export,
        )?;
        validate_request_body_export(request_content, request_body_export)?;
        let assembly = self.assembly.to_config()?;
        let stream_classifier = self.stream_classifier.to_config()?;
        if stream_classifier.soft_sniff_max_bytes > assembly.max_buffer_bytes {
            return Err(format!(
                "semantic_retention.l0_llm_call.stream_classifier.soft_sniff_max_bytes ({}) must not exceed semantic_retention.l0_llm_call.assembly.max_buffer_bytes ({})",
                stream_classifier.soft_sniff_max_bytes, assembly.max_buffer_bytes
            ));
        }
        Ok(L0LlmCallRetention {
            enabled: self.enabled,
            request_content,
            request_body_export,
            request_body_export_max_bytes: require_positive_u64(
                "semantic_retention.l0_llm_call.request_body_export_max_bytes",
                self.request_body_export_max_bytes,
            )?,
            response_content: parse_value(
                "semantic_retention.l0_llm_call.response_content",
                &self.response_content,
            )?,
            tool_calls: parse_value(
                "semantic_retention.l0_llm_call.tool_calls",
                &self.tool_calls,
            )?,
            tool_result_content_export: parse_value(
                "semantic_retention.l0_llm_call.tool_result_content_export",
                &self.tool_result_content_export,
            )?,
            tool_result_content_export_max_bytes: require_positive_u64(
                "semantic_retention.l0_llm_call.tool_result_content_export_max_bytes",
                self.tool_result_content_export_max_bytes,
            )?,
            usage: parse_value("semantic_retention.l0_llm_call.usage", &self.usage)?,
            retain_assembled_payload: self.retain_assembled_payload,
            websocket_max_connections_per_process: require_positive_u32(
                "semantic_retention.l0_llm_call.websocket_max_connections_per_process",
                self.websocket_max_connections_per_process,
            )?,
            assembly,
            stream_classifier,
            projection_state: self.projection_state.to_config()?,
            trajectory: self.trajectory.to_config()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LlmProjectionStateDocument {
    pub max_pending_requests_per_stream: u32,
    pub max_pending_responses_per_stream: u32,
    pub max_action_versions_per_trace: u32,
    pub max_pending_trajectory_actions_per_trace: u32,
    pub max_tool_entries_per_trace: u32,
    pub max_active_response_bindings_per_trace: u32,
    pub max_damaged_response_bindings_per_trace: u32,
    pub max_correlation_streams_per_trace: u32,
}

impl Default for LlmProjectionStateDocument {
    fn default() -> Self {
        Self {
            max_pending_requests_per_stream: DEFAULT_LLM_PROJECTION_MAX_PENDING_REQUESTS_PER_STREAM,
            max_pending_responses_per_stream:
                DEFAULT_LLM_PROJECTION_MAX_PENDING_RESPONSES_PER_STREAM,
            max_action_versions_per_trace: DEFAULT_LLM_PROJECTION_MAX_ACTION_VERSIONS_PER_TRACE,
            max_pending_trajectory_actions_per_trace:
                DEFAULT_LLM_PROJECTION_MAX_PENDING_TRAJECTORY_ACTIONS_PER_TRACE,
            max_tool_entries_per_trace: DEFAULT_LLM_PROJECTION_MAX_TOOL_ENTRIES_PER_TRACE,
            max_active_response_bindings_per_trace:
                DEFAULT_LLM_PROJECTION_MAX_ACTIVE_RESPONSE_BINDINGS_PER_TRACE,
            max_damaged_response_bindings_per_trace:
                DEFAULT_LLM_PROJECTION_MAX_DAMAGED_RESPONSE_BINDINGS_PER_TRACE,
            max_correlation_streams_per_trace:
                DEFAULT_LLM_PROJECTION_MAX_CORRELATION_STREAMS_PER_TRACE,
        }
    }
}

impl LlmProjectionStateDocument {
    fn from_config(config: &LlmProjectionStateConfig) -> Self {
        Self {
            max_pending_requests_per_stream: config.max_pending_requests_per_stream,
            max_pending_responses_per_stream: config.max_pending_responses_per_stream,
            max_action_versions_per_trace: config.max_action_versions_per_trace,
            max_pending_trajectory_actions_per_trace: config
                .max_pending_trajectory_actions_per_trace,
            max_tool_entries_per_trace: config.max_tool_entries_per_trace,
            max_active_response_bindings_per_trace: config.max_active_response_bindings_per_trace,
            max_damaged_response_bindings_per_trace: config.max_damaged_response_bindings_per_trace,
            max_correlation_streams_per_trace: config.max_correlation_streams_per_trace,
        }
    }

    fn to_config(&self) -> Result<LlmProjectionStateConfig, String> {
        self.validate_usize(
            "semantic_retention.l0_llm_call.projection_state.max_pending_requests_per_stream",
            self.max_pending_requests_per_stream,
        )?;
        self.validate_usize(
            "semantic_retention.l0_llm_call.projection_state.max_pending_responses_per_stream",
            self.max_pending_responses_per_stream,
        )?;
        self.validate_usize(
            "semantic_retention.l0_llm_call.projection_state.max_action_versions_per_trace",
            self.max_action_versions_per_trace,
        )?;
        self.validate_usize(
            "semantic_retention.l0_llm_call.projection_state.max_pending_trajectory_actions_per_trace",
            self.max_pending_trajectory_actions_per_trace,
        )?;
        self.validate_usize(
            "semantic_retention.l0_llm_call.projection_state.max_tool_entries_per_trace",
            self.max_tool_entries_per_trace,
        )?;
        self.validate_usize(
            "semantic_retention.l0_llm_call.projection_state.max_active_response_bindings_per_trace",
            self.max_active_response_bindings_per_trace,
        )?;
        self.validate_usize(
            "semantic_retention.l0_llm_call.projection_state.max_damaged_response_bindings_per_trace",
            self.max_damaged_response_bindings_per_trace,
        )?;
        self.validate_usize(
            "semantic_retention.l0_llm_call.projection_state.max_correlation_streams_per_trace",
            self.max_correlation_streams_per_trace,
        )?;
        Ok(LlmProjectionStateConfig {
            max_pending_requests_per_stream: self.max_pending_requests_per_stream,
            max_pending_responses_per_stream: self.max_pending_responses_per_stream,
            max_action_versions_per_trace: self.max_action_versions_per_trace,
            max_pending_trajectory_actions_per_trace: self.max_pending_trajectory_actions_per_trace,
            max_tool_entries_per_trace: self.max_tool_entries_per_trace,
            max_active_response_bindings_per_trace: self.max_active_response_bindings_per_trace,
            max_damaged_response_bindings_per_trace: self.max_damaged_response_bindings_per_trace,
            max_correlation_streams_per_trace: self.max_correlation_streams_per_trace,
        })
    }

    fn validate_usize(&self, path: &'static str, value: u32) -> Result<(), String> {
        let value = require_positive_u32(path, value)?;
        usize::try_from(value)
            .map(|_| ())
            .map_err(|error| format!("{path} must fit usize: {error}"))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LlmStreamClassifierDocument {
    pub soft_sniff_max_bytes: u64,
}

impl Default for LlmStreamClassifierDocument {
    fn default() -> Self {
        Self {
            soft_sniff_max_bytes: DEFAULT_LLM_STREAM_CLASSIFIER_SOFT_SNIFF_MAX_BYTES,
        }
    }
}

impl LlmStreamClassifierDocument {
    fn from_config(config: &LlmStreamClassifierConfig) -> Self {
        Self {
            soft_sniff_max_bytes: config.soft_sniff_max_bytes,
        }
    }

    fn to_config(&self) -> Result<LlmStreamClassifierConfig, String> {
        let soft_sniff_max_bytes = require_positive_u64(
            "semantic_retention.l0_llm_call.stream_classifier.soft_sniff_max_bytes",
            self.soft_sniff_max_bytes,
        )?;
        usize::try_from(soft_sniff_max_bytes).map_err(|error| {
            format!(
                "semantic_retention.l0_llm_call.stream_classifier.soft_sniff_max_bytes must fit usize: {error}"
            )
        })?;
        Ok(LlmStreamClassifierConfig {
            soft_sniff_max_bytes,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LlmAssemblyDocument {
    pub max_buffer_bytes: u64,
    pub max_segment_ranges: u32,
}

impl Default for LlmAssemblyDocument {
    fn default() -> Self {
        Self::from_config(&LlmAssemblyConfig::default())
    }
}

impl LlmAssemblyDocument {
    fn from_config(config: &LlmAssemblyConfig) -> Self {
        Self {
            max_buffer_bytes: config.max_buffer_bytes,
            max_segment_ranges: config.max_segment_ranges,
        }
    }

    fn to_config(&self) -> Result<LlmAssemblyConfig, String> {
        let max_buffer_bytes = require_positive_u64(
            "semantic_retention.l0_llm_call.assembly.max_buffer_bytes",
            self.max_buffer_bytes,
        )?;
        usize::try_from(max_buffer_bytes).map_err(|error| {
            format!(
                "semantic_retention.l0_llm_call.assembly.max_buffer_bytes must fit usize: {error}"
            )
        })?;
        let max_segment_ranges = require_positive_u32(
            "semantic_retention.l0_llm_call.assembly.max_segment_ranges",
            self.max_segment_ranges,
        )?;
        usize::try_from(max_segment_ranges).map_err(|error| {
            format!(
                "semantic_retention.l0_llm_call.assembly.max_segment_ranges must fit usize: {error}"
            )
        })?;
        Ok(LlmAssemblyConfig {
            max_buffer_bytes,
            max_segment_ranges,
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

/// Exporting a body the host does not retain is self-contradictory: there is
/// no canonical body to send. Reject the combination at startup rather than
/// letting a deployment run for days before anyone notices.
fn validate_request_body_export(
    request_content: LlmRequestContentRetention,
    request_body_export: LlmRequestBodyExportRetention,
) -> Result<(), String> {
    if matches!(request_body_export, LlmRequestBodyExportRetention::None) {
        return Ok(());
    }
    if matches!(request_content, LlmRequestContentRetention::CanonicalBlocks) {
        return Ok(());
    }
    Err(format!(
        "semantic_retention.l0_llm_call.request_body_export = \"{}\" requires \
         semantic_retention.l0_llm_call.request_content = \"canonical_blocks\", got \"{}\"",
        llm_request_body_export_retention_as_str(request_body_export),
        llm_request_content_retention_as_str(request_content)
    ))
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
    pub exchange: HttpExchangeDocument,
}

impl Default for L2HttpDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            message_summary: true,
            headers: "metadata".to_string(),
            body_content: "text".to_string(),
            exchange: HttpExchangeDocument::default(),
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
            exchange: self.exchange.to_config()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct HttpExchangeDocument {
    pub max_pending_requests_per_stream: u32,
    pub max_pending_responses_per_stream: u32,
    pub response_lateness: String,
}

impl Default for HttpExchangeDocument {
    fn default() -> Self {
        Self::from_config(&HttpExchangeConfig::default())
    }
}

impl HttpExchangeDocument {
    fn from_config(config: &HttpExchangeConfig) -> Self {
        Self {
            max_pending_requests_per_stream: config.max_pending_requests_per_stream,
            max_pending_responses_per_stream: config.max_pending_responses_per_stream,
            response_lateness: duration_as_string(config.response_lateness),
        }
    }

    fn to_config(&self) -> Result<HttpExchangeConfig, String> {
        Ok(HttpExchangeConfig {
            max_pending_requests_per_stream: require_positive_u32(
                "semantic_retention.l2_http.exchange.max_pending_requests_per_stream",
                self.max_pending_requests_per_stream,
            )?,
            max_pending_responses_per_stream: require_positive_u32(
                "semantic_retention.l2_http.exchange.max_pending_responses_per_stream",
                self.max_pending_responses_per_stream,
            )?,
            response_lateness: parse_required_duration(
                "semantic_retention.l2_http.exchange.response_lateness",
                &self.response_lateness,
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
