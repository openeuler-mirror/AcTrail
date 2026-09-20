use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct L0LlmCallDocument {
    pub enabled: bool,
    pub request_content: String,
    pub request_body_export: String,
    pub request_body_export_max_bytes: u64,
    pub response_content: String,
    pub tool_calls: String,
    pub tool_results_enabled: bool,
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
            tool_results_enabled: true,
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
    pub(in crate::daemon::operator::document) fn to_config(
        &self,
    ) -> Result<L0LlmCallRetention, String> {
        let request_content = parse_value(
            "semantic_retention.l0_llm_call.request_content",
            &self.request_content,
        )?;
        let request_body_export = parse_value(
            "semantic_retention.l0_llm_call.request_body_export",
            &self.request_body_export,
        )?;
        validate_request_body_export(request_content, request_body_export)?;
        let tool_result_content_export = parse_value(
            "semantic_retention.l0_llm_call.tool_result_content_export",
            &self.tool_result_content_export,
        )?;
        if !self.tool_results_enabled
            && tool_result_content_export != LlmToolResultContentExportRetention::None
        {
            return Err("semantic_retention.l0_llm_call.tool_result_content_export requires semantic_retention.l0_llm_call.tool_results_enabled = true".to_string());
        }
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
            tool_results_enabled: self.tool_results_enabled,
            tool_result_content_export,
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
