//! Anthropic streaming response assembly.

use semantic_action::{
    LlmParsedResponse, LlmParsedSseEvent, LlmProviderResponseParser,
    LlmProviderResponseStreamParser, LlmSseEvent, LlmTokenUsage,
};

use super::anthropic::{ANTHROPIC_PROVIDER_ID, AnthropicMessagesResponseParser};
use super::common::ToolCallAssembler;

#[derive(Default)]
pub(super) struct AnthropicMessagesStreamParser {
    parsed_events: Vec<LlmParsedSseEvent>,
}

impl LlmProviderResponseStreamParser for AnthropicMessagesStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        let parsed = AnthropicMessagesResponseParser.parse_sse_event(event);
        self.parsed_events.push(parsed.clone());
        parsed
    }

    fn finish(&mut self) -> Option<LlmParsedResponse> {
        parsed_events_to_response(ANTHROPIC_PROVIDER_ID, &self.parsed_events, None, true)
    }
}

pub(super) fn parsed_events_to_response(
    provider_id: &'static str,
    parsed_events: &[LlmParsedSseEvent],
    token_usage: Option<LlmTokenUsage>,
    stream: bool,
) -> Option<LlmParsedResponse> {
    let content_chunks = parsed_events
        .iter()
        .filter_map(|event| event.content_text.clone())
        .collect::<Vec<_>>();
    let reasoning_chunks = parsed_events
        .iter()
        .filter_map(|event| event.reasoning_text.clone())
        .collect::<Vec<_>>();
    let mut assembler = ToolCallAssembler::default();
    for event in parsed_events {
        for tool_call in &event.tool_calls {
            assembler.apply_call_delta(tool_call.clone());
        }
    }
    let tool_calls = assembler.into_calls();
    let done = parsed_events.iter().any(|event| event.done);
    if content_chunks.is_empty() && reasoning_chunks.is_empty() && tool_calls.is_empty() && !done {
        return None;
    }
    let content_text = (!content_chunks.is_empty()).then(|| content_chunks.join(""));
    let reasoning_text = (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join(""));
    Some(LlmParsedResponse {
        provider_id,
        model: parsed_events.iter().find_map(|event| event.model.clone()),
        output_text: content_text.clone().or_else(|| reasoning_text.clone()),
        content_text,
        reasoning_text,
        tool_calls,
        token_usage,
        chunk_count: content_chunks.len() + reasoning_chunks.len(),
        done,
        stream,
    })
}
