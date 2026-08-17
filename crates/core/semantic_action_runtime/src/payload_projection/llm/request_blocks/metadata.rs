use serde_json::Value;

use super::canonical_json;

const MESSAGE_PREVIEW_MAX_CHARS: usize = 160;

pub(in crate::payload_projection::llm) struct UserMessageMetadata {
    pub(in crate::payload_projection::llm) count: usize,
    pub(in crate::payload_projection::llm) latest_hash: Option<String>,
}

pub(super) fn message_preview(body: &Value) -> Option<String> {
    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        if let Some(preview) = latest_user_message_preview(messages) {
            return Some(preview);
        }
    }
    if let Some(input) = body.get("input").and_then(Value::as_array)
        && let Some(preview) = latest_user_message_preview(input)
    {
        return Some(preview);
    }
    let mut parts = Vec::new();
    collect_text(body.get("input").unwrap_or(&Value::Null), &mut parts);
    if parts.is_empty() {
        collect_text(body.get("prompt").unwrap_or(&Value::Null), &mut parts);
    }
    preview_from_parts(parts)
}

pub(super) fn user_message_metadata(body: &Value) -> UserMessageMetadata {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .or_else(|| body.get("input").and_then(Value::as_array));
    let Some(messages) = messages else {
        return UserMessageMetadata {
            count: 0,
            latest_hash: None,
        };
    };
    let user_messages = messages
        .iter()
        .filter(|message| message_is_user_input(message))
        .collect::<Vec<_>>();
    UserMessageMetadata {
        count: user_messages.len(),
        latest_hash: user_messages
            .last()
            .map(|message| canonical_json::sha256_hex(&canonical_json::bytes(message))),
    }
}

pub(super) fn background_request_kind(body: &Value) -> Option<&'static str> {
    let messages = body.get("messages").and_then(Value::as_array)?;
    let mut system_parts = Vec::new();
    for message in messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("system"))
    {
        collect_text(message.get("content").unwrap_or(message), &mut system_parts);
    }
    let system_text = system_parts.join(" ").to_ascii_lowercase();
    if system_text.contains("title generator")
        && (system_text.contains("thread title")
            || system_text.contains("title for this conversation")
            || system_text.contains("find this conversation later"))
    {
        return Some("title_generation");
    }
    if system_text.contains("conversation summarizer")
        || (system_text.contains("summarize the conversation")
            && system_text.contains("output only"))
    {
        return Some("conversation_summary");
    }
    None
}

fn latest_user_message_preview(messages: &[Value]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        if !message_is_user_input(message) {
            return None;
        }
        let mut parts = Vec::new();
        collect_text(message.get("content").unwrap_or(message), &mut parts);
        preview_from_parts(parts)
    })
}

fn preview_from_parts(parts: Vec<String>) -> Option<String> {
    let joined = parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let preview = truncate_chars(joined.trim(), MESSAGE_PREVIEW_MAX_CHARS);
    (!preview.is_empty()).then_some(preview)
}

fn message_is_user_input(message: &Value) -> bool {
    let Some(role) = message.get("role").and_then(Value::as_str) else {
        return false;
    };
    if role == "human" {
        return true;
    }
    role == "user"
        && !message
            .get("content")
            .is_some_and(content_is_only_tool_results)
}

fn content_is_only_tool_results(content: &Value) -> bool {
    match content {
        Value::Array(blocks) => !blocks.is_empty() && blocks.iter().all(block_is_tool_result),
        Value::Object(_) => block_is_tool_result(content),
        _ => false,
    }
}

fn block_is_tool_result(block: &Value) -> bool {
    block
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "tool_result" | "tool-result"))
}

fn collect_text(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(text) => parts.push(text.clone()),
        Value::Array(values) => {
            for value in values {
                collect_text(value, parts);
            }
        }
        Value::Object(object) => {
            for key in ["text", "content", "input"] {
                if let Some(value) = object.get(key) {
                    collect_text(value, parts);
                }
            }
        }
        _ => {}
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            output.push_str("...");
            break;
        }
        output.push(ch);
    }
    output
}
