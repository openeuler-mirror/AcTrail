//! Generic JSON LLM request parser.

use semantic_action::LlmProviderMatch;
use serde_json::Value;

use super::request_registry::{LlmRequestParser, LlmRequestParserInput, ParsedLlmRequest};

pub(super) const GENERIC_JSON_REQUEST_CLASSIFIER_ID: &str = "generic-json-request";

#[derive(Default)]
pub(super) struct GenericJsonRequestParser;

impl LlmRequestParser for GenericJsonRequestParser {
    fn classifier_id(&self) -> &'static str {
        GENERIC_JSON_REQUEST_CLASSIFIER_ID
    }

    fn match_json_request(&self, input: &LlmRequestParserInput<'_>) -> LlmProviderMatch {
        if generic_json_request(input.json) {
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
            model: input
                .json
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        })
    }
}

fn generic_json_request(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.contains_key("model")
        && (object.contains_key("messages")
            || object.contains_key("prompt")
            || object.contains_key("input"))
}
