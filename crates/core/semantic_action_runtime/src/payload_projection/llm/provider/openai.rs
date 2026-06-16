//! OpenAI-compatible response parser.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmProviderMatch,
    LlmProviderResponseParser, LlmProviderResponseStreamParser, LlmSseEvent, LlmSseResponseInput,
};
use serde_json::Value;

use super::common::{
    ToolCallAssembler, extract_finish_reason, extract_response_texts, extract_token_usage,
    extract_token_usage_from_values,
};

pub(super) const OPENAI_COMPATIBLE_PROVIDER_ID: &str = "openai-compatible";

#[derive(Default)]
pub(super) struct OpenAiCompatibleResponseParser;

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

    fn parse_json_response(&self, input: LlmJsonResponseInput<'_>) -> Option<LlmParsedResponse> {
        if self.match_json_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let texts = extract_response_texts(input.json);
        let chunk_count = texts.chunk_count();
        let mut assembler = ToolCallAssembler::default();
        assembler.apply_value(input.json);
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
            token_usage: extract_token_usage(input.json),
            chunk_count,
            done: true,
            stream: false,
        })
    }

    fn parse_sse_response(&self, input: LlmSseResponseInput<'_>) -> Option<LlmParsedResponse> {
        if self.match_sse_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let parsed_events = input
            .events
            .iter()
            .map(|event| self.parse_sse_event(*event))
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
        assembler.apply_values(input.events.iter().filter_map(|event| event.json));
        let tool_calls = assembler.into_calls();
        let done = parsed_events.iter().any(|event| event.done);
        if content_chunks.is_empty()
            && reasoning_chunks.is_empty()
            && tool_calls.is_empty()
            && !done
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
            token_usage: extract_token_usage_from_values(
                input.events.iter().filter_map(|event| event.json),
            ),
            chunk_count: content_chunks.len() + reasoning_chunks.len(),
            done,
            stream: true,
        })
    }

    fn parse_sse_event(&self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        if event.done_marker {
            return LlmParsedSseEvent {
                done: true,
                ..LlmParsedSseEvent::default()
            };
        }
        let Some(value) = event.json else {
            return LlmParsedSseEvent::default();
        };
        let texts = extract_response_texts(value);
        let mut assembler = ToolCallAssembler::default();
        assembler.apply_value(value);
        LlmParsedSseEvent {
            model: value
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content_text: texts.content_text,
            reasoning_text: texts.reasoning_text,
            tool_calls: assembler.into_calls(),
            done: event.done_marker,
            finish_reason: extract_finish_reason(value),
        }
    }

    fn new_stream_parser(&self) -> Box<dyn LlmProviderResponseStreamParser + Send> {
        Box::new(OpenAiCompatibleStreamParser::default())
    }
}

#[derive(Default)]
struct OpenAiCompatibleStreamParser {
    parsed_events: Vec<LlmParsedSseEvent>,
}

impl LlmProviderResponseStreamParser for OpenAiCompatibleStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        let parsed = OpenAiCompatibleResponseParser.parse_sse_event(event);
        self.parsed_events.push(parsed.clone());
        parsed
    }

    fn finish(&mut self) -> Option<LlmParsedResponse> {
        let content_chunks = self
            .parsed_events
            .iter()
            .filter_map(|event| event.content_text.clone())
            .collect::<Vec<_>>();
        let reasoning_chunks = self
            .parsed_events
            .iter()
            .filter_map(|event| event.reasoning_text.clone())
            .collect::<Vec<_>>();
        let mut assembler = ToolCallAssembler::default();
        for event in &self.parsed_events {
            for tool_call in &event.tool_calls {
                assembler.apply_call_delta(tool_call.clone());
            }
        }
        let tool_calls = assembler.into_calls();
        let done = self.parsed_events.iter().any(|event| event.done);
        if content_chunks.is_empty()
            && reasoning_chunks.is_empty()
            && tool_calls.is_empty()
            && !done
        {
            return None;
        }
        let content_text = (!content_chunks.is_empty()).then(|| content_chunks.join(""));
        let reasoning_text = (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join(""));
        Some(LlmParsedResponse {
            provider_id: OPENAI_COMPATIBLE_PROVIDER_ID,
            model: self
                .parsed_events
                .iter()
                .find_map(|event| event.model.clone()),
            content_text,
            reasoning_text,
            tool_calls,
            token_usage: None,
            chunk_count: content_chunks.len() + reasoning_chunks.len(),
            done,
            stream: true,
        })
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
