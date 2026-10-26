//! Generic JSON LLM request parser.

use semantic_action::LlmProviderMatch;

use super::{LlmRequestParser, LlmRequestParserInput, ParsedLlmRequest};

pub(super) const GENERIC_JSON_REQUEST_CLASSIFIER_ID: &str = "generic-json-request";

#[derive(Default)]
pub(super) struct GenericJsonRequestParser;

impl LlmRequestParser for GenericJsonRequestParser {
    fn classifier_id(&self) -> &'static str {
        GENERIC_JSON_REQUEST_CLASSIFIER_ID
    }

    fn match_json_request(&self, input: &LlmRequestParserInput<'_>) -> LlmProviderMatch {
        if input.facts.generic_match() {
            LlmProviderMatch::Plausible
        } else {
            LlmProviderMatch::NoMatch
        }
    }

    fn parse_json_request(&self, input: &LlmRequestParserInput<'_>) -> Option<ParsedLlmRequest> {
        if self.match_json_request(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        Some(ParsedLlmRequest {
            classifier_id: self.classifier_id(),
            protocol_id: None,
            model: input.facts.model.clone(),
        })
    }
}
