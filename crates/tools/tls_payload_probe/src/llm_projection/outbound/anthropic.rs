use serde_json::Value;

use crate::capture::{AssembledHttp, CaptureDirection};

use super::super::model::{LlmOutput, LlmRequestItem, LlmRequestSchema, request};
use super::common::{
    JSON_FIELD_MESSAGES, content_text, message_items, model, stream, string_field,
};

const PATH_MESSAGES: &str = "/messages";
const JSON_FIELD_SYSTEM: &str = "system";
const LABEL_SYSTEM: &str = "system";

#[derive(Debug, Default)]
pub(in crate::llm_projection) struct AnthropicMessagesRequestParser;

impl AnthropicMessagesRequestParser {
    pub(in crate::llm_projection) fn parse(
        message: &AssembledHttp,
        value: &Value,
    ) -> Option<LlmOutput> {
        if message.direction != CaptureDirection::Outbound || !matches_request(message, value) {
            return None;
        }
        let mut items = Vec::new();
        if let Some(system) = system_text(value) {
            items.push(LlmRequestItem {
                label: LABEL_SYSTEM.to_string(),
                text: system,
            });
        }
        items.extend(message_items(value));
        if items.is_empty() {
            return None;
        }
        Some(request(
            message.pid,
            message.stream_key,
            message.direction,
            LlmRequestSchema::AnthropicMessages,
            model(value),
            stream(value),
            items,
        ))
    }
}

fn matches_request(message: &AssembledHttp, value: &Value) -> bool {
    message.first_line.contains(PATH_MESSAGES)
        && value
            .get(JSON_FIELD_MESSAGES)
            .and_then(Value::as_array)
            .is_some_and(|messages| !messages.is_empty())
}

fn system_text(value: &Value) -> Option<String> {
    string_field(value, JSON_FIELD_SYSTEM)
        .or_else(|| value.get(JSON_FIELD_SYSTEM).and_then(content_text))
}
