use super::*;

impl SemanticRetentionConfig {
    pub fn llm_layer_enabled(&self) -> bool {
        self.l0_llm_call.enabled
    }

    pub fn llm_request_consumed_by_l0(&self) -> bool {
        self.l0_llm_call.enabled
            && !matches!(
                self.l0_llm_call.request_content,
                LlmRequestContentRetention::None
            )
    }

    /// Whether the canonical request body is carried on the action for
    /// export. Configuration validation already guarantees this is only ever
    /// true alongside `canonical_blocks` request retention.
    pub fn llm_request_body_export_enabled(&self) -> bool {
        self.l0_llm_call.enabled
            && matches!(
                self.l0_llm_call.request_body_export,
                LlmRequestBodyExportRetention::CanonicalJson
            )
    }

    pub fn llm_response_assembled_provider_enabled(&self) -> bool {
        self.l0_llm_call.enabled
            && matches!(
                self.l0_llm_call.response_content,
                LlmResponseContentRetention::AssembledProvider
            )
    }

    pub fn llm_response_tool_calls_enabled(&self) -> bool {
        self.l0_llm_call.enabled
            && matches!(
                self.l0_llm_call.tool_calls,
                LlmToolCallRetention::AssembledJson
            )
    }

    pub fn llm_tool_result_content_export_enabled(&self) -> bool {
        self.l0_llm_call.enabled
            && matches!(
                self.l0_llm_call.tool_result_content_export,
                LlmToolResultContentExportRetention::CanonicalJson
            )
    }

    pub fn llm_response_usage_enabled(&self) -> bool {
        self.l0_llm_call.enabled && matches!(self.l0_llm_call.usage, LlmUsageRetention::Summary)
    }

    pub fn llm_trajectory_enabled(&self) -> bool {
        self.l0_llm_call.enabled
            && self.l0_llm_call.trajectory.enabled
            && matches!(
                self.l0_llm_call.request_content,
                LlmRequestContentRetention::CanonicalBlocks
            )
    }

    pub fn sse_stream_summary_enabled(&self) -> bool {
        self.l1_sse.enabled && self.l1_sse.stream_summary
    }

    pub fn sse_event_content_for_llm_response(&self) -> SseEventContentRetention {
        if !self.l1_sse.enabled {
            return SseEventContentRetention::None;
        }
        if self.content_owner == SemanticContentOwner::HighestConsumed
            && self.llm_response_assembled_provider_enabled()
        {
            return SseEventContentRetention::None;
        }
        self.l1_sse.event_content
    }

    pub fn http_message_summary_enabled(&self) -> bool {
        self.l2_http.enabled && self.l2_http.message_summary
    }

    pub fn http_headers(&self) -> HttpHeadersRetention {
        if self.l2_http.enabled {
            self.l2_http.headers
        } else {
            HttpHeadersRetention::None
        }
    }

    pub fn http_body_content_for_http_message(&self, llm_message: bool) -> HttpBodyRetention {
        if !self.l2_http.enabled {
            return HttpBodyRetention::None;
        }
        if llm_message
            && self.content_owner == SemanticContentOwner::HighestConsumed
            && (self.llm_request_consumed_by_l0() || self.llm_response_assembled_provider_enabled())
        {
            return HttpBodyRetention::None;
        }
        self.l2_http.body_content
    }

    pub fn http_body_content_needed(&self, llm_message: bool) -> bool {
        !matches!(
            self.http_body_content_for_http_message(llm_message),
            HttpBodyRetention::None
        )
    }

    pub fn http2_frame_summary_enabled(&self) -> bool {
        self.l3_http2_frame.enabled && self.l3_http2_frame.frame_summary
    }

    pub fn http2_data_content(&self) -> Http2DataContentRetention {
        if self.l3_http2_frame.enabled {
            self.l3_http2_frame.data_content
        } else {
            Http2DataContentRetention::None
        }
    }

    pub fn retain_transport_payload_body(&self, consumed_by_higher_layer: bool) -> bool {
        if !self.l4_payload.enabled {
            return false;
        }
        if !matches!(
            self.l4_payload.body_content,
            PayloadBodyContentRetention::Retained
        ) {
            return false;
        }
        self.content_owner == SemanticContentOwner::ConfiguredLayers || !consumed_by_higher_layer
    }
}
