//! OpenAI Responses API SSE parser.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmProviderMatch,
    LlmProviderResponseParser, LlmProviderResponseStreamParser, LlmSseEvent, LlmSseResponseInput,
    LlmTokenUsage,
};
use serde_json::Value;

use super::common::{ParsedSseResponseAccumulator, extract_token_usage_from_values};

const OPENAI_RESPONSES_PROVIDER_ID: &str = "openai-responses";
const RESPONSE_EVENT_PREFIX: &str = "response.";

#[derive(Default)]
pub(super) struct OpenAiResponsesResponseParser;

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

    fn parse_json_response(&self, _input: LlmJsonResponseInput<'_>) -> Option<LlmParsedResponse> {
        None
    }

    fn parse_sse_response(&self, input: LlmSseResponseInput<'_>) -> Option<LlmParsedResponse> {
        if self.match_sse_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let mut accumulator = ParsedSseResponseAccumulator::default();
        for event in input.events {
            accumulator.observe(&self.parse_sse_event(*event));
        }
        accumulator.finish(self.provider_id(), Self::token_usage(input.events), true)
    }

    fn parse_sse_event(&self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        let Some(value) = event.json else {
            return LlmParsedSseEvent {
                done: event.done_marker,
                ..LlmParsedSseEvent::default()
            };
        };
        let event_type = Self::event_type(event, value);
        let response = value.get("response");
        LlmParsedSseEvent {
            model: response
                .and_then(|response| response.get("model"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content_text: (event_type == Some("response.output_text.delta"))
                .then(|| value.get("delta").and_then(Value::as_str))
                .flatten()
                .filter(|delta| !delta.is_empty())
                .map(ToString::to_string),
            done: event.done_marker || event_type == Some("response.completed"),
            finish_reason: (event_type == Some("response.completed"))
                .then(|| {
                    response
                        .and_then(|response| response.get("status"))
                        .and_then(Value::as_str)
                })
                .flatten()
                .map(ToString::to_string),
            ..LlmParsedSseEvent::default()
        }
    }

    fn new_stream_parser(&self) -> Box<dyn LlmProviderResponseStreamParser + Send> {
        Box::new(OpenAiResponsesStreamParser::default())
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
}

#[derive(Default)]
struct OpenAiResponsesStreamParser {
    accumulator: ParsedSseResponseAccumulator,
    token_usage: Option<LlmTokenUsage>,
}

impl LlmProviderResponseStreamParser for OpenAiResponsesStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        if let Some(usage) = event
            .json
            .and_then(|value| value.get("response"))
            .and_then(super::common::extract_token_usage)
        {
            self.token_usage = Some(usage);
        }
        let parsed = OpenAiResponsesResponseParser.parse_sse_event(event);
        self.accumulator.observe(&parsed);
        parsed
    }

    fn finish(&mut self) -> Option<LlmParsedResponse> {
        self.accumulator
            .finish(OPENAI_RESPONSES_PROVIDER_ID, self.token_usage.clone(), true)
    }
}
