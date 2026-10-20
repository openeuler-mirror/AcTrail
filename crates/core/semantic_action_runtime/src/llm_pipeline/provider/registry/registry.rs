//! Provider parser selection.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmProviderMatch, LlmProviderResponseParser,
    LlmProviderResponseStreamParser, LlmResponseRetention, LlmSseResponseInput,
};

use crate::llm_pipeline::provider::anthropic::AnthropicMessagesResponseParser;
use crate::llm_pipeline::provider::openai_chat::OpenAiCompatibleResponseParser;
use crate::llm_pipeline::provider::openai_responses::OpenAiResponsesResponseParser;
use crate::llm_pipeline::provider::structured_json::StructuredJsonSseParser;

static OPENAI_COMPATIBLE: OpenAiCompatibleResponseParser = OpenAiCompatibleResponseParser;
static OPENAI_RESPONSES: OpenAiResponsesResponseParser = OpenAiResponsesResponseParser;
static ANTHROPIC_MESSAGES: AnthropicMessagesResponseParser = AnthropicMessagesResponseParser;
static STRUCTURED_JSON_SSE: StructuredJsonSseParser = StructuredJsonSseParser;
static PARSERS: &[&dyn LlmProviderResponseParser] = &[
    &OPENAI_RESPONSES,
    &STRUCTURED_JSON_SSE,
    &ANTHROPIC_MESSAGES,
    &OPENAI_COMPATIBLE,
];

pub(in crate::llm_pipeline) fn new_responses_stream_parser(
    retention: LlmResponseRetention,
) -> Box<dyn LlmProviderResponseStreamParser + Send> {
    OPENAI_RESPONSES.new_stream_parser(retention)
}

pub(in crate::llm_pipeline) fn parse_json_response(
    input: LlmJsonResponseInput<'_>,
    retention: LlmResponseRetention,
) -> Option<LlmParsedResponse> {
    select_json_parser(input)?.parse_json_response(input, retention)
}

/// Create a stateful incremental SSE stream parser for the provider selected
/// by the given events.
///
/// Both incremental consumption and batch recovery use this parser so they
/// share the same provider final boundary.
pub(in crate::llm_pipeline) fn new_sse_stream_parser(
    input: LlmSseResponseInput<'_>,
    retention: LlmResponseRetention,
) -> Option<(
    Box<dyn LlmProviderResponseStreamParser + Send>,
    &'static str,
)> {
    select_sse_parser(input)
        .map(|parser| (parser.new_stream_parser(retention), parser.provider_id()))
}

fn select_json_parser(
    input: LlmJsonResponseInput<'_>,
) -> Option<&'static dyn LlmProviderResponseParser> {
    select_parser(|parser| parser.match_json_response(input))
}

fn select_sse_parser(
    input: LlmSseResponseInput<'_>,
) -> Option<&'static dyn LlmProviderResponseParser> {
    select_parser(|parser| parser.match_sse_response(input))
}

fn select_parser(
    matcher: impl Fn(&'static dyn LlmProviderResponseParser) -> LlmProviderMatch,
) -> Option<&'static dyn LlmProviderResponseParser> {
    let mut selected = None;
    let mut selected_match = LlmProviderMatch::NoMatch;
    let mut ambiguous = false;
    for &parser in parsers() {
        let candidate_match = matcher(parser);
        if candidate_match == LlmProviderMatch::NoMatch {
            continue;
        }
        if candidate_match > selected_match {
            selected = Some(parser);
            selected_match = candidate_match;
            ambiguous = false;
        } else if candidate_match == selected_match {
            ambiguous = true;
        }
    }
    if ambiguous { None } else { selected }
}

fn parsers() -> &'static [&'static dyn LlmProviderResponseParser] {
    PARSERS
}
