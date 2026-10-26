use config_core::daemon::SemanticRetentionConfig;
use semantic_action::{
    LlmParsedResponse, LlmProviderResponseStreamParser, LlmResponseRetention,
    LlmResponseTermination, LlmSseEvent, LlmSseResponseInput, SemanticAction,
    SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus, attr_keys as attrs,
};
use serde_json::Value;

use crate::llm_pipeline::projection::ProjectionBatch;
use crate::llm_pipeline::provider::{
    new_responses_stream_parser, new_sse_stream_parser, tool_calls_json,
};
use crate::llm_pipeline::stream::response::LlmResponseBody;

use super::super::response::ProjectedProviderResponseId;
use super::super::support::{insert_response_content_attributes, llm_response_title};
use super::ParsedMessageContext;

/// Provider content state; transport ownership and request binding stay with the caller.
pub(in crate::llm_pipeline) struct ParsedProviderStream {
    parser: Box<dyn LlmProviderResponseStreamParser + Send>,
    provider_id: &'static str,
    response_id: Option<String>,
    termination: Option<LlmResponseTermination>,
    event_count: usize,
}

impl ParsedProviderStream {
    pub(in crate::llm_pipeline) fn responses(config: &SemanticRetentionConfig) -> Self {
        Self {
            parser: new_responses_stream_parser(LlmResponseRetention {
                content: config.llm_response_assembled_provider_enabled(),
                tool_calls: config.llm_response_tool_calls_enabled(),
                usage: config.llm_response_usage_enabled(),
            }),
            provider_id: "openai-responses",
            response_id: None,
            termination: None,
            event_count: 0,
        }
    }
    /// Select the provider and consume the first already parsed event exactly once.
    pub(in crate::llm_pipeline) fn new(
        config: &SemanticRetentionConfig,
        value: &Value,
    ) -> Option<Self> {
        if !config.llm_layer_enabled() {
            return None;
        }
        let event = Self::event(value, 0);
        let (parser, provider_id) = new_sse_stream_parser(
            LlmSseResponseInput {
                text: "",
                events: &[event],
            },
            LlmResponseRetention {
                content: config.llm_response_assembled_provider_enabled(),
                tool_calls: config.llm_response_tool_calls_enabled(),
                usage: config.llm_response_usage_enabled(),
            },
        )?;
        let mut stream = Self {
            parser,
            provider_id,
            response_id: None,
            termination: None,
            event_count: 0,
        };
        stream.observe(value);
        Some(stream)
    }

    pub(in crate::llm_pipeline) fn observe(&mut self, value: &Value) {
        if self.terminal() {
            return;
        }
        let parsed = self
            .parser
            .observe_event(Self::event(value, self.event_count));
        self.event_count += 1;
        if let Some(id) = value
            .get("response")
            .and_then(|response| response.get("id"))
            .and_then(Value::as_str)
        {
            self.response_id = Some(id.to_owned());
        }
        self.termination = parsed.termination.or(self.termination);
    }

    pub(in crate::llm_pipeline) fn terminal(&self) -> bool {
        self.termination.is_some()
    }

    pub(in crate::llm_pipeline) fn snapshot(
        &mut self,
        config: &SemanticRetentionConfig,
        context: &ParsedMessageContext,
    ) -> ProjectionBatch {
        let parsed = self.parser.finish().unwrap_or_else(|| LlmParsedResponse {
            provider_id: self.provider_id,
            stream: true,
            ..LlmParsedResponse::default()
        });
        let termination = self.termination.or(parsed.termination);
        let body = LlmResponseBody {
            provider_id: self.provider_id.into(),
            provider_response_id: self.response_id.clone(),
            json_valid: true,
            model: parsed.model,
            content_text: parsed.content_text,
            reasoning_text: parsed.reasoning_text,
            tool_calls_json: config
                .llm_response_tool_calls_enabled()
                .then(|| tool_calls_json(&parsed.tool_calls))
                .flatten(),
            token_usage: parsed.token_usage,
            chunk_count: parsed.chunk_count,
            termination,
            stream: true,
        };
        let mut attributes = context.attributes();
        insert_response_content_attributes(config, &body, &mut attributes);
        attributes.insert(attrs::llm_response::BODY_FORMAT.into(), "json".into());
        attributes.insert(
            attrs::llm_response::PAYLOAD_BYTES.into(),
            context.payload_bytes.to_string(),
        );
        let status = match termination {
            Some(reason) if reason.is_failure() => SemanticActionStatus::Error,
            Some(_) => SemanticActionStatus::Success,
            None => SemanticActionStatus::InProgress,
        };
        ProjectionBatch {
            actions: vec![SemanticAction {
                action_id: context.action_id.clone(),
                trace_id: context.identity.trace_id,
                process: context.identity.process,
                kind: SemanticActionKind::LlmResponse,
                title: llm_response_title(&attributes),
                start_time: context.start_at,
                end_time: termination.map(|_| context.last_at),
                status,
                completeness: if termination.is_some() {
                    SemanticActionCompleteness::Complete
                } else {
                    SemanticActionCompleteness::Partial
                },
                attributes,
                evidence: context.evidence.clone(),
            }],
            provider_response_ids: self
                .response_id
                .as_ref()
                .map(|id| ProjectedProviderResponseId {
                    action_id: context.action_id.clone(),
                    provider_response_id: id.clone(),
                })
                .into_iter()
                .collect(),
            ..ProjectionBatch::default()
        }
    }

    fn event(value: &Value, index: usize) -> LlmSseEvent<'_> {
        // The provider parser consumes JSON directly. No SSE framing is synthesized.
        LlmSseEvent {
            index,
            event_type: value.get("type").and_then(Value::as_str),
            id: None,
            data: "",
            json: Some(value),
            done_marker: false,
        }
    }
}
