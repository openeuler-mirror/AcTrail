//! Request-side LLM protocol parser selection.

use semantic_action::LlmProviderMatch;
use serde_json::Value;

use super::generic_request::GenericJsonRequestParser;
use super::structured_json_sse::StructuredJsonSseParser;

static GENERIC_JSON_REQUEST: GenericJsonRequestParser = GenericJsonRequestParser;
static STRUCTURED_JSON_SSE: StructuredJsonSseParser = StructuredJsonSseParser;
static PARSERS: &[&dyn LlmRequestParser] = &[&STRUCTURED_JSON_SSE, &GENERIC_JSON_REQUEST];

pub(in crate::payload_projection::llm) struct LlmRequestParserInput<'a> {
    pub(in crate::payload_projection::llm) json: &'a Value,
}

pub(in crate::payload_projection::llm) struct ParsedLlmRequest {
    pub(in crate::payload_projection::llm) classifier_id: &'static str,
    pub(in crate::payload_projection::llm) protocol_id: Option<&'static str>,
    pub(in crate::payload_projection::llm) model: Option<String>,
}

pub(super) trait LlmRequestParser: Send + Sync {
    fn classifier_id(&self) -> &'static str;

    fn match_json_request(&self, input: &LlmRequestParserInput<'_>) -> LlmProviderMatch;

    fn parse_json_request(&self, input: &LlmRequestParserInput<'_>) -> Option<ParsedLlmRequest>;
}

pub(in crate::payload_projection::llm) fn parse_json_request(
    input: &LlmRequestParserInput<'_>,
) -> Option<ParsedLlmRequest> {
    select_parser(input)?.parse_json_request(input)
}

fn select_parser(input: &LlmRequestParserInput<'_>) -> Option<&'static dyn LlmRequestParser> {
    let mut selected = None;
    let mut selected_match = LlmProviderMatch::NoMatch;
    let mut ambiguous = false;
    for &parser in parsers() {
        let candidate_match = parser.match_json_request(input);
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

fn parsers() -> &'static [&'static dyn LlmRequestParser] {
    PARSERS
}
