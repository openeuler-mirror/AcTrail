//! Anthropic streaming response assembly.

use semantic_action::{
    LlmParsedResponse, LlmParsedSseEvent, LlmProviderResponseParser,
    LlmProviderResponseStreamParser, LlmResponseRetention, LlmSseEvent, LlmTokenUsage,
};

use super::parser::{ANTHROPIC_PROVIDER_ID, AnthropicMessagesResponseParser};
use crate::llm_pipeline::provider::driver::{ParsedSseResponseAccumulator, ToolCallAssembler};

#[derive(Default)]
pub(super) struct AnthropicMessagesStreamParser {
    retention: LlmResponseRetention,
    accumulator: ParsedSseResponseAccumulator,
}

impl AnthropicMessagesStreamParser {
    pub(super) fn new(retention: LlmResponseRetention) -> Self {
        Self {
            retention,
            ..Default::default()
        }
    }
}

impl LlmProviderResponseStreamParser for AnthropicMessagesStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        let parsed = AnthropicMessagesResponseParser.parse_sse_event(event, self.retention);
        self.accumulator.observe(&parsed);
        parsed
    }

    fn finish(&mut self) -> Option<LlmParsedResponse> {
        self.accumulator.finish(ANTHROPIC_PROVIDER_ID, None, true)
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
    let termination = parsed_events
        .iter()
        .filter_map(|event| event.termination)
        .max();
    if !parsed_events.iter().any(|event| event.response_observed) && termination.is_none() {
        return None;
    }
    let content_text = (!content_chunks.is_empty()).then(|| content_chunks.join(""));
    let reasoning_text = (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join(""));
    Some(LlmParsedResponse {
        provider_id,
        model: parsed_events.iter().find_map(|event| event.model.clone()),
        content_text,
        reasoning_text,
        tool_calls,
        token_usage,
        chunk_count: parsed_events
            .iter()
            .map(|event| event.text_chunk_count)
            .sum(),
        termination,
        stream,
    })
}
