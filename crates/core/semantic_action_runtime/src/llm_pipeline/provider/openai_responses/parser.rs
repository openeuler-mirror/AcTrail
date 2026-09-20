//! OpenAI Responses API SSE parser.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmProviderMatch,
    LlmProviderResponseParser, LlmProviderResponseStreamParser, LlmResponseRetention,
    LlmResponseTermination, LlmSseEvent, LlmSseResponseInput, LlmTokenUsage, LlmToolCall,
    LlmToolFunction,
};
use serde_json::Value;

use crate::llm_pipeline::provider::driver::{
    ParsedSseResponseAccumulator, ToolCallAssembler, extract_token_usage_from_values,
    qualified_response_tool_name,
};

const OPENAI_RESPONSES_PROVIDER_ID: &str = "openai-responses";
const RESPONSE_EVENT_PREFIX: &str = "response.";

#[derive(Default)]
pub(in crate::llm_pipeline) struct OpenAiResponsesResponseParser;

impl LlmProviderResponseParser for OpenAiResponsesResponseParser {
    fn provider_id(&self) -> &'static str {
        OPENAI_RESPONSES_PROVIDER_ID
    }

    fn match_json_response(&self, _input: LlmJsonResponseInput<'_>) -> LlmProviderMatch {
        LlmProviderMatch::NoMatch
    }

    fn match_sse_response(&self, input: LlmSseResponseInput<'_>) -> LlmProviderMatch {
        if input.events.iter().any(Self::is_responses_event) {
            LlmProviderMatch::Strong
        } else {
            LlmProviderMatch::NoMatch
        }
    }

    fn parse_json_response(
        &self,
        _input: LlmJsonResponseInput<'_>,
        _retention: LlmResponseRetention,
    ) -> Option<LlmParsedResponse> {
        None
    }

    fn parse_sse_response(
        &self,
        input: LlmSseResponseInput<'_>,
        retention: LlmResponseRetention,
    ) -> Option<LlmParsedResponse> {
        if self.match_sse_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let mut accumulator = ParsedSseResponseAccumulator::default();
        for event in input.events {
            accumulator.observe(&self.parse_sse_event(*event, retention));
        }
        accumulator.finish(
            self.provider_id(),
            retention
                .usage
                .then(|| Self::token_usage(input.events))
                .flatten(),
            true,
        )
    }

    fn parse_sse_event(
        &self,
        event: LlmSseEvent<'_>,
        retention: LlmResponseRetention,
    ) -> LlmParsedSseEvent {
        let Some(value) = event.json else {
            return LlmParsedSseEvent {
                termination: event
                    .done_marker
                    .then_some(LlmResponseTermination::Completed),
                ..LlmParsedSseEvent::default()
            };
        };
        let event_type = Self::event_type(event, value);
        let response = value.get("response");
        let text = (event_type == Some("response.output_text.delta"))
            .then(|| value.get("delta").and_then(Value::as_str))
            .flatten()
            .filter(|delta| !delta.is_empty());
        let tool_observed = matches!(
            event_type,
            Some("response.output_item.done" | "response.output_item.added")
        ) && value.get("item").is_some_and(|item| {
            matches!(
                item.get("type").and_then(Value::as_str),
                Some("function_call" | "custom_tool_call")
            ) && item
                .get("call_id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
        });
        LlmParsedSseEvent {
            text_chunk_count: usize::from(text.is_some()),
            response_observed: text.is_some() || tool_observed,
            model: response
                .and_then(|response| response.get("model"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content_text: text.filter(|_| retention.content).map(ToString::to_string),
            termination: match event_type {
                Some("response.failed")
                    if response
                        .and_then(|response| response.get("status"))
                        .and_then(Value::as_str)
                        == Some("failed") =>
                {
                    Some(LlmResponseTermination::Failed)
                }
                Some("response.incomplete")
                    if response
                        .and_then(|response| response.get("status"))
                        .and_then(Value::as_str)
                        == Some("incomplete") =>
                {
                    Some(LlmResponseTermination::Incomplete)
                }
                Some("error") if value.get("message").and_then(Value::as_str).is_some() => {
                    Some(LlmResponseTermination::Failed)
                }
                Some("response.completed") => Some(LlmResponseTermination::Completed),
                _ => event
                    .done_marker
                    .then_some(LlmResponseTermination::Completed),
            },
            finish_reason: (event_type == Some("response.completed"))
                .then(|| {
                    response
                        .and_then(|response| response.get("status"))
                        .and_then(Value::as_str)
                })
                .flatten()
                .map(ToString::to_string),
            tool_calls: if retention.tool_calls {
                Self::tool_calls_for_event(event_type, value)
            } else {
                Vec::new()
            },
            ..LlmParsedSseEvent::default()
        }
    }

    fn new_stream_parser(
        &self,
        retention: LlmResponseRetention,
    ) -> Box<dyn LlmProviderResponseStreamParser + Send> {
        Box::new(OpenAiResponsesStreamParser {
            retention,
            ..Default::default()
        })
    }
}

impl OpenAiResponsesResponseParser {
    fn event_type<'a>(event: LlmSseEvent<'a>, value: &'a Value) -> Option<&'a str> {
        value
            .get("type")
            .and_then(Value::as_str)
            .or(event.event_type)
    }

    fn is_responses_event(event: &LlmSseEvent<'_>) -> bool {
        event
            .json
            .and_then(|value| Self::event_type(*event, value))
            .is_some_and(|event_type| event_type.starts_with(RESPONSE_EVENT_PREFIX))
    }

    fn token_usage(events: &[LlmSseEvent<'_>]) -> Option<LlmTokenUsage> {
        extract_token_usage_from_values(
            events
                .iter()
                .filter_map(|event| event.json)
                .filter_map(|value| value.get("response")),
        )
    }

    /// Extract tool_call deltas from OpenAI Responses API SSE events.
    ///
    /// Primary source is `response.output_item.done`, whose `item` carries the
    /// authoritative finalized function_call (name, call_id, arguments,
    /// namespace) — per OpenAI guidance, deltas are redelivered in the `done`
    /// event, so we skip them to avoid double-accumulation. Falls back to
    /// `response.output_item.added` for providers that emit the complete item
    /// up front. The accumulator merges updates by `call_id`.
    fn tool_calls_for_event(event_type: Option<&str>, value: &Value) -> Vec<LlmToolCall> {
        let Some(event_type) = event_type else {
            return Vec::new();
        };
        let is_done = event_type == "response.output_item.done";
        let is_added = event_type == "response.output_item.added";
        if !is_done && !is_added {
            return Vec::new();
        }
        let Some(item) = value.get("item").and_then(Value::as_object) else {
            return Vec::new();
        };
        if item.get("type").and_then(Value::as_str) == Some("custom_tool_call") {
            let mut assembler = ToolCallAssembler::default();
            assembler.apply_openai_response_custom_tool_call(item);
            return assembler.into_calls();
        }
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return Vec::new();
        }
        let call_id = item.get("call_id").and_then(Value::as_str);
        let name = qualified_response_tool_name(item);
        // `added` often carries empty arguments; wait for `done` to capture the
        // finalized string and avoid storing a stale empty value.
        let arguments = if is_done {
            item.get("arguments").and_then(Value::as_str)
        } else {
            None
        };
        build_tool_call(call_id, name.as_deref(), arguments)
    }
}

fn build_tool_call(
    call_id: Option<&str>,
    name: Option<&str>,
    arguments: Option<&str>,
) -> Vec<LlmToolCall> {
    let function = LlmToolFunction {
        name: name.map(ToString::to_string),
        arguments: arguments.map(ToString::to_string),
        arguments_json: None,
    };
    if function.name.is_none() && function.arguments.is_none() && call_id.is_none() {
        return Vec::new();
    }
    vec![LlmToolCall {
        index: None,
        id: call_id.map(ToString::to_string),
        kind: Some("function".to_string()),
        function: Some(function),
    }]
}

#[derive(Default)]
struct OpenAiResponsesStreamParser {
    retention: LlmResponseRetention,
    accumulator: ParsedSseResponseAccumulator,
    token_usage: Option<LlmTokenUsage>,
}

impl LlmProviderResponseStreamParser for OpenAiResponsesStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        if let Some(usage) = event
            .json
            .filter(|_| self.retention.usage)
            .and_then(|value| value.get("response"))
            .and_then(crate::llm_pipeline::provider::driver::extract_token_usage)
        {
            self.token_usage = Some(usage);
        }
        let parsed = OpenAiResponsesResponseParser.parse_sse_event(event, self.retention);
        self.accumulator.observe(&parsed);
        parsed
    }

    fn finish(&mut self) -> Option<LlmParsedResponse> {
        self.accumulator
            .finish(OPENAI_RESPONSES_PROVIDER_ID, self.token_usage.clone(), true)
    }
}
