use serde_json::Value;

use crate::capture::{AssembledHttp, CaptureDirection};

use super::super::model::{LlmOutput, LlmRequestItem, LlmRequestSchema, request};
use super::common::{
    JSON_FIELD_INPUT, JSON_FIELD_INSTRUCTIONS, JSON_FIELD_ROLE, content_text, input_item_text,
    joined_text, model, stream, string_field,
};

const PATH_RESPONSES: &str = "/responses";
const LABEL_INPUT: &str = "input";
const LABEL_INSTRUCTIONS: &str = "instructions";

#[derive(Debug, Default)]
pub(in crate::llm_projection) struct ResponsesRequestParser;

impl ResponsesRequestParser {
    pub(in crate::llm_projection) fn parse(
        message: &AssembledHttp,
        value: &Value,
    ) -> Option<LlmOutput> {
        if message.direction != CaptureDirection::Outbound
            || !message.first_line.contains(PATH_RESPONSES)
        {
            return None;
        }
        let mut items = Vec::new();
        if let Some(instructions) = string_field(value, JSON_FIELD_INSTRUCTIONS) {
            items.push(LlmRequestItem {
                label: LABEL_INSTRUCTIONS.to_string(),
                text: instructions,
            });
        }
        items.extend(input_items(value));
        if items.is_empty() {
            return None;
        }
        Some(request(
            message.pid,
            message.stream_key,
            message.direction,
            LlmRequestSchema::OpenAiResponses,
            model(value),
            stream(value),
            items,
        ))
    }
}

fn input_items(value: &Value) -> Vec<LlmRequestItem> {
    let Some(input) = value.get(JSON_FIELD_INPUT) else {
        return Vec::new();
    };
    match input {
        Value::String(text) if !text.is_empty() => vec![LlmRequestItem {
            label: LABEL_INPUT.to_string(),
            text: text.clone(),
        }],
        Value::Array(items) => items.iter().filter_map(input_array_item).collect(),
        Value::Object(_) => input_item_text(input)
            .map(|text| {
                vec![LlmRequestItem {
                    label: LABEL_INPUT.to_string(),
                    text,
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn input_array_item(value: &Value) -> Option<LlmRequestItem> {
    let label = value
        .get(JSON_FIELD_ROLE)
        .and_then(Value::as_str)
        .unwrap_or(LABEL_INPUT)
        .to_string();
    let text = input_item_text(value).or_else(|| {
        value
            .as_array()
            .and_then(|items| joined_text(items.iter().filter_map(content_text)))
    })?;
    Some(LlmRequestItem { label, text })
}
