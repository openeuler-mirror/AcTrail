//! OpenAI-compatible response parser.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmProviderMatch,
    LlmProviderResponseParser, LlmProviderResponseStreamParser, LlmResponseRetention,
    LlmResponseTermination, LlmSseEvent, LlmSseResponseInput,
};
use serde_json::Value;

use crate::llm_pipeline::provider::driver::{
    ParsedSseResponseAccumulator, ResponseTexts, ToolCallAssembler, extract_finish_reason,
    extract_token_usage, extract_token_usage_from_values,
};

pub(in crate::llm_pipeline) const OPENAI_COMPATIBLE_PROVIDER_ID: &str = "openai-compatible";

#[derive(Default)]
pub(in crate::llm_pipeline) struct OpenAiCompatibleResponseParser;

impl LlmProviderResponseParser for OpenAiCompatibleResponseParser {
    fn provider_id(&self) -> &'static str {
        OPENAI_COMPATIBLE_PROVIDER_ID
    }

    fn match_json_response(&self, input: LlmJsonResponseInput<'_>) -> LlmProviderMatch {
        openai_json_match(input.json)
    }

    fn match_sse_response(&self, input: LlmSseResponseInput<'_>) -> LlmProviderMatch {
        let mut best = LlmProviderMatch::NoMatch;
        for event in input.events {
            if let Some(value) = event.json {
                best = best.max(openai_json_match(value));
            }
        }
        best
    }

    fn parse_json_response(
        &self,
        input: LlmJsonResponseInput<'_>,
        retention: LlmResponseRetention,
    ) -> Option<LlmParsedResponse> {
        if self.match_json_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let texts = ResponseTexts::extract(input.json, retention.content);
        let chunk_count = texts.chunk_count();
        let mut assembler = ToolCallAssembler::default();
        if retention.tool_calls {
            assembler.apply_value(input.json);
        }
        Some(LlmParsedResponse {
            provider_id: self.provider_id(),
            model: input
                .json
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content_text: texts.content_text,
            reasoning_text: texts.reasoning_text,
            tool_calls: assembler.into_calls(),
            token_usage: retention
                .usage
                .then(|| extract_token_usage(input.json))
                .flatten(),
            chunk_count,
            termination: Some(LlmResponseTermination::Completed),
            stream: false,
        })
    }

    fn parse_sse_response(
        &self,
        input: LlmSseResponseInput<'_>,
        retention: LlmResponseRetention,
    ) -> Option<LlmParsedResponse> {
        if self.match_sse_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let parsed_events = input
            .events
            .iter()
            .map(|event| self.parse_sse_event(*event, retention))
            .collect::<Vec<_>>();
        let content_chunks = parsed_events
            .iter()
            .filter_map(|event| event.content_text.clone())
            .collect::<Vec<_>>();
        let reasoning_chunks = parsed_events
            .iter()
            .filter_map(|event| event.reasoning_text.clone())
            .collect::<Vec<_>>();
        let mut assembler = ToolCallAssembler::default();
        if retention.tool_calls {
            assembler.apply_values(input.events.iter().filter_map(|event| event.json));
        }
        let tool_calls = assembler.into_calls();
        let termination = parsed_events
            .iter()
            .filter_map(|event| event.termination)
            .max();
        if !parsed_events.iter().any(|event| event.response_observed)
            && tool_calls.is_empty()
            && termination.is_none()
        {
            return None;
        }
        let content_text = (!content_chunks.is_empty()).then(|| content_chunks.join(""));
        let reasoning_text = (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join(""));
        Some(LlmParsedResponse {
            provider_id: self.provider_id(),
            model: parsed_events.iter().find_map(|event| event.model.clone()),
            content_text,
            reasoning_text,
            tool_calls,
            token_usage: retention
                .usage
                .then(|| {
                    extract_token_usage_from_values(
                        input.events.iter().filter_map(|event| event.json),
                    )
                })
                .flatten(),
            chunk_count: parsed_events
                .iter()
                .map(|event| event.text_chunk_count)
                .sum(),
            termination,
            stream: true,
        })
    }

    fn parse_sse_event(
        &self,
        event: LlmSseEvent<'_>,
        retention: LlmResponseRetention,
    ) -> LlmParsedSseEvent {
        if event.done_marker {
            return LlmParsedSseEvent {
                termination: Some(LlmResponseTermination::Completed),
                ..LlmParsedSseEvent::default()
            };
        }
        let Some(value) = event.json else {
            return LlmParsedSseEvent::default();
        };
        let texts = ResponseTexts::extract(value, retention.content);
        let text_chunk_count = texts.chunk_count();
        let response_observed = text_chunk_count > 0 || texts.tool_observed;
        let finish_reason = extract_finish_reason(value);
        let mut assembler = ToolCallAssembler::default();
        if retention.tool_calls {
            assembler.apply_value(value);
        }
        LlmParsedSseEvent {
            text_chunk_count,
            response_observed,
            model: value
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content_text: texts.content_text,
            reasoning_text: texts.reasoning_text,
            tool_calls: assembler.into_calls(),
            termination: if Self::has_error(value) {
                Some(LlmResponseTermination::Failed)
            } else {
                finish_reason
                    .is_some()
                    .then_some(LlmResponseTermination::Completed)
            },
            finish_reason,
        }
    }

    fn new_stream_parser(
        &self,
        retention: LlmResponseRetention,
    ) -> Box<dyn LlmProviderResponseStreamParser + Send> {
        Box::new(OpenAiCompatibleStreamParser {
            retention,
            ..Default::default()
        })
    }
}

impl OpenAiCompatibleResponseParser {
    fn has_error(value: &Value) -> bool {
        value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .is_some()
    }
}

#[derive(Default)]
struct OpenAiCompatibleStreamParser {
    retention: LlmResponseRetention,
    accumulator: ParsedSseResponseAccumulator,
}

impl LlmProviderResponseStreamParser for OpenAiCompatibleStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        let parsed = OpenAiCompatibleResponseParser.parse_sse_event(event, self.retention);
        self.accumulator.observe(&parsed);
        parsed
    }

    fn finish(&mut self) -> Option<LlmParsedResponse> {
        self.accumulator
            .finish(OPENAI_COMPATIBLE_PROVIDER_ID, None, true)
    }
}

fn openai_json_match(value: &Value) -> LlmProviderMatch {
    let Some(object) = value.as_object() else {
        return LlmProviderMatch::NoMatch;
    };
    if !object.contains_key("model") {
        return LlmProviderMatch::NoMatch;
    }
    if object.contains_key("choices") || object.contains_key("output") {
        return LlmProviderMatch::Strong;
    }
    if object.contains_key("content") {
        return LlmProviderMatch::Plausible;
    }
    LlmProviderMatch::NoMatch
}
