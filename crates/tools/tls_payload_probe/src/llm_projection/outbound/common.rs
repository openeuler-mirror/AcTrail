use serde_json::Value;

use super::super::model::LlmRequestItem;

pub(super) const JSON_FIELD_CONTENT: &str = "content";
pub(super) const JSON_FIELD_INPUT: &str = "input";
pub(super) const JSON_FIELD_INSTRUCTIONS: &str = "instructions";
pub(super) const JSON_FIELD_MESSAGES: &str = "messages";
pub(super) const JSON_FIELD_MODEL: &str = "model";
pub(super) const JSON_FIELD_ROLE: &str = "role";
pub(super) const JSON_FIELD_STREAM: &str = "stream";
pub(super) const JSON_FIELD_TEXT: &str = "text";
pub(super) const JSON_FIELD_TYPE: &str = "type";
pub(super) const JSON_TYPE_INPUT_TEXT: &str = "input_text";
pub(super) const JSON_TYPE_TEXT: &str = "text";

pub(super) fn model(value: &Value) -> Option<String> {
    value
        .get(JSON_FIELD_MODEL)
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .map(ToString::to_string)
}

pub(super) fn stream(value: &Value) -> Option<bool> {
    value.get(JSON_FIELD_STREAM).and_then(Value::as_bool)
}

pub(super) fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

pub(super) fn message_items(value: &Value) -> Vec<LlmRequestItem> {
    let Some(messages) = value.get(JSON_FIELD_MESSAGES).and_then(Value::as_array) else {
        return Vec::new();
    };
    messages
        .iter()
        .filter_map(|message| {
            let role = message
                .get(JSON_FIELD_ROLE)
                .and_then(Value::as_str)
                .unwrap_or("message");
            let text = content_text(message.get(JSON_FIELD_CONTENT)?)?;
            Some(LlmRequestItem {
                label: role.to_string(),
                text,
            })
        })
        .collect()
}

pub(super) fn content_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Array(items) => joined_text(items.iter().filter_map(content_block_text)),
        Value::Object(_) => content_block_text(value),
        _ => None,
    }
}

pub(super) fn content_block_text(value: &Value) -> Option<String> {
    if let Some(text) = value
        .get(JSON_FIELD_TEXT)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }
    if let Some(content) = value.get(JSON_FIELD_CONTENT) {
        return content_text(content);
    }
    None
}

pub(super) fn input_item_text(value: &Value) -> Option<String> {
    if let Some(text) = content_block_text(value) {
        return Some(text);
    }
    match value.get(JSON_FIELD_TYPE).and_then(Value::as_str) {
        Some(JSON_TYPE_INPUT_TEXT | JSON_TYPE_TEXT) => value
            .get(JSON_FIELD_TEXT)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string),
        _ => None,
    }
}

pub(super) fn joined_text<'a>(parts: impl Iterator<Item = String> + 'a) -> Option<String> {
    let text = parts.collect::<Vec<_>>().join("");
    if text.is_empty() { None } else { Some(text) }
}
