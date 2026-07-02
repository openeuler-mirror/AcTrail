//! Agent semantic-action configuration.

use std::str::FromStr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentInvocationConfig {
    pub enabled: bool,
    pub commands: Vec<String>,
}

impl Default for AgentInvocationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            commands: Vec::new(),
        }
    }
}

impl AgentInvocationConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticRetentionConfig {
    pub content_owner: SemanticContentOwner,
    pub l0_llm_call: L0LlmCallRetention,
    pub l1_sse: L1SseRetention,
    pub l2_http: L2HttpRetention,
    pub l3_http2_frame: L3Http2FrameRetention,
    pub l4_payload: L4PayloadRetention,
}

impl Default for SemanticRetentionConfig {
    fn default() -> Self {
        Self {
            content_owner: SemanticContentOwner::HighestConsumed,
            l0_llm_call: L0LlmCallRetention::default(),
            l1_sse: L1SseRetention::default(),
            l2_http: L2HttpRetention::default(),
            l3_http2_frame: L3Http2FrameRetention::default(),
            l4_payload: L4PayloadRetention::default(),
        }
    }
}

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

    pub fn llm_response_usage_enabled(&self) -> bool {
        self.l0_llm_call.enabled && matches!(self.l0_llm_call.usage, LlmUsageRetention::Summary)
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SemanticContentOwner {
    #[default]
    HighestConsumed,
    ConfiguredLayers,
}

impl FromStr for SemanticContentOwner {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "highest_consumed" => Ok(Self::HighestConsumed),
            "configured_layers" => Ok(Self::ConfiguredLayers),
            other => Err(format!("unsupported semantic content owner {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L0LlmCallRetention {
    pub enabled: bool,
    pub request_content: LlmRequestContentRetention,
    pub response_content: LlmResponseContentRetention,
    pub tool_calls: LlmToolCallRetention,
    pub usage: LlmUsageRetention,
}

impl Default for L0LlmCallRetention {
    fn default() -> Self {
        Self {
            enabled: true,
            request_content: LlmRequestContentRetention::CanonicalBlocks,
            response_content: LlmResponseContentRetention::AssembledProvider,
            tool_calls: LlmToolCallRetention::AssembledJson,
            usage: LlmUsageRetention::Summary,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LlmRequestContentRetention {
    None,
    Shape,
    #[default]
    CanonicalBlocks,
}

impl FromStr for LlmRequestContentRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "shape" => Ok(Self::Shape),
            "canonical_blocks" => Ok(Self::CanonicalBlocks),
            other => Err(format!("unsupported LLM request content retention {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LlmResponseContentRetention {
    None,
    #[default]
    AssembledProvider,
}

impl FromStr for LlmResponseContentRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "assembled_provider" => Ok(Self::AssembledProvider),
            other => Err(format!(
                "unsupported LLM response content retention {other}"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LlmToolCallRetention {
    None,
    #[default]
    AssembledJson,
}

impl FromStr for LlmToolCallRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "assembled_json" => Ok(Self::AssembledJson),
            other => Err(format!("unsupported LLM tool call retention {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LlmUsageRetention {
    None,
    #[default]
    Summary,
}

impl FromStr for LlmUsageRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "summary" => Ok(Self::Summary),
            other => Err(format!("unsupported LLM usage retention {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L1SseRetention {
    pub enabled: bool,
    pub stream_summary: bool,
    pub event_content: SseEventContentRetention,
}

impl Default for L1SseRetention {
    fn default() -> Self {
        Self {
            enabled: true,
            stream_summary: true,
            event_content: SseEventContentRetention::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SseEventContentRetention {
    #[default]
    None,
    Parsed,
    Raw,
}

impl FromStr for SseEventContentRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "parsed" => Ok(Self::Parsed),
            "raw" => Ok(Self::Raw),
            other => Err(format!("unsupported SSE event content retention {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L2HttpRetention {
    pub enabled: bool,
    pub message_summary: bool,
    pub headers: HttpHeadersRetention,
    pub body_content: HttpBodyRetention,
}

impl Default for L2HttpRetention {
    fn default() -> Self {
        Self {
            enabled: true,
            message_summary: true,
            headers: HttpHeadersRetention::Metadata,
            body_content: HttpBodyRetention::Text,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HttpHeadersRetention {
    None,
    #[default]
    Metadata,
    Full,
}

impl FromStr for HttpHeadersRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "metadata" => Ok(Self::Metadata),
            "full" => Ok(Self::Full),
            other => Err(format!("unsupported HTTP headers retention {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HttpBodyRetention {
    None,
    #[default]
    Text,
    Json,
    Raw,
}

impl FromStr for HttpBodyRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "raw" => Ok(Self::Raw),
            other => Err(format!("unsupported HTTP body retention {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L3Http2FrameRetention {
    pub enabled: bool,
    pub frame_summary: bool,
    pub data_content: Http2DataContentRetention,
}

impl Default for L3Http2FrameRetention {
    fn default() -> Self {
        Self {
            enabled: true,
            frame_summary: true,
            data_content: Http2DataContentRetention::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Http2DataContentRetention {
    #[default]
    None,
    Preview,
    Raw,
}

impl FromStr for Http2DataContentRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "preview" => Ok(Self::Preview),
            "raw" => Ok(Self::Raw),
            other => Err(format!("unsupported HTTP/2 DATA retention {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L4PayloadRetention {
    pub enabled: bool,
    pub stats: bool,
    pub body_content: PayloadBodyContentRetention,
}

impl Default for L4PayloadRetention {
    fn default() -> Self {
        Self {
            enabled: true,
            stats: true,
            body_content: PayloadBodyContentRetention::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PayloadBodyContentRetention {
    #[default]
    None,
    Retained,
}

impl FromStr for PayloadBodyContentRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "retained" => Ok(Self::Retained),
            other => Err(format!("unsupported payload body retention {other}")),
        }
    }
}
