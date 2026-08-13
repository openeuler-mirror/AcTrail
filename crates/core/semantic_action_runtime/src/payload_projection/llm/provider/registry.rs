//! Provider parser selection.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmProviderMatch, LlmProviderResponseParser,
    LlmProviderResponseStreamParser, LlmSseResponseInput,
};

use super::anthropic::AnthropicMessagesResponseParser;
use super::openai::OpenAiCompatibleResponseParser;
use super::openai_responses::OpenAiResponsesResponseParser;
use super::structured_json_sse::StructuredJsonSseParser;

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

pub(in crate::payload_projection::llm) struct ParsedSseResponse {
    pub(in crate::payload_projection::llm) response: LlmParsedResponse,
}

pub(in crate::payload_projection::llm) fn parse_json_response(
    input: LlmJsonResponseInput<'_>,
) -> Option<LlmParsedResponse> {
    select_json_parser(input)?.parse_json_response(input)
}

pub(in crate::payload_projection::llm) fn parse_sse_response(
    input: LlmSseResponseInput<'_>,
) -> Option<ParsedSseResponse> {
    let parser = select_sse_parser(input)?;
    let response = parser.parse_sse_response(input)?;
    Some(ParsedSseResponse { response })
}

/// Create a stateful incremental SSE stream parser for the provider selected
/// by the given events.
///
/// The incremental parser mirrors the batch `parse_sse_response` aggregation
/// but consumes one event at a time, so a growing SSE body only needs its
/// newly-appended events parsed instead of re-parsing the entire stream.
pub(in crate::payload_projection::llm) fn new_sse_stream_parser(
    input: LlmSseResponseInput<'_>,
) -> Option<(
    Box<dyn LlmProviderResponseStreamParser + Send>,
    &'static str,
)> {
    select_sse_parser(input).map(|parser| (parser.new_stream_parser(), parser.provider_id()))
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
