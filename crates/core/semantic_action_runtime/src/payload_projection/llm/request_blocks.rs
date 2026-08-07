use std::collections::BTreeMap;
use std::fmt::Write as _;

use model_core::ids::TraceId;
use semantic_action::{
    LlmRequestBlock, LlmRequestBlockRef, LlmRequestContentWrite, LlmRequestManifest,
};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

pub(super) const FORMAT_VERSION: u32 = 1;

const BLOCK_PLACEHOLDER_KEY: &str = "$actrail_llm_block";
const MESSAGE_PREVIEW_MAX_CHARS: usize = 160;

pub(super) struct CanonicalRequestContent {
    pub(super) write: LlmRequestContentWrite,
    pub(super) canonical_body_hash: String,
    pub(super) canonical_body_bytes: u64,
    pub(super) block_count: usize,
    pub(super) message_preview: Option<String>,
    pub(super) user_message_count: usize,
    pub(super) latest_user_message_hash: Option<String>,
    pub(super) background_kind: Option<&'static str>,
}

pub(super) struct UserMessageMetadata {
    pub(super) count: usize,
    pub(super) latest_hash: Option<String>,
}

pub(super) fn canonical_request_content(
    trace_id: TraceId,
    action_id: &str,
    body: &Value,
) -> Result<CanonicalRequestContent, String> {
    let user_messages = user_message_metadata(body);
    let canonical_body = canonical_json_bytes(body);
    let canonical_body_hash = sha256_hex(&canonical_body);
    let canonical_body_bytes = canonical_body.len() as u64;
    let mut accumulator = BlockAccumulator::new(trace_id, action_id);
    let skeleton = skeletonize_body(body, &mut accumulator)?;
    let skeleton_json = canonical_json_string(&skeleton);
    let (block_refs, blocks) = accumulator.into_parts();
    let manifest = LlmRequestManifest {
        trace_id,
        action_id: action_id.to_string(),
        format_version: FORMAT_VERSION,
        canonical_body_hash: canonical_body_hash.clone(),
        canonical_body_bytes,
        skeleton_json,
    };
    let block_count = block_refs.len();
    Ok(CanonicalRequestContent {
        write: LlmRequestContentWrite {
            manifest,
            block_refs,
            blocks,
        },
        canonical_body_hash,
        canonical_body_bytes,
        block_count,
        message_preview: message_preview(body),
        user_message_count: user_messages.count,
        latest_user_message_hash: user_messages.latest_hash,
        background_kind: background_request_kind(body),
    })
}

pub(super) fn canonical_shape_metadata(
    body: &Value,
) -> (
    String,
    u64,
    Option<String>,
    UserMessageMetadata,
    Option<&'static str>,
) {
    let canonical_body = canonical_json_bytes(body);
    (
        sha256_hex(&canonical_body),
        canonical_body.len() as u64,
        message_preview(body),
        user_message_metadata(body),
        background_request_kind(body),
    )
}

fn skeletonize_body(body: &Value, accumulator: &mut BlockAccumulator) -> Result<Value, String> {
    let Some(object) = body.as_object() else {
        return Ok(body.clone());
    };
    let mut skeleton = Map::new();
    for (key, value) in object {
        let next = match key.as_str() {
            "tools" => skeletonize_array_items(value, accumulator)?,
            "messages" => skeletonize_messages(value, accumulator)?,
            "prompt" => accumulator.add_block(value)?,
            "input" => skeletonize_input(value, accumulator)?,
            _ => value.clone(),
        };
        skeleton.insert(key.clone(), next);
    }
    Ok(Value::Object(skeleton))
}

fn skeletonize_array_items(
    value: &Value,
    accumulator: &mut BlockAccumulator,
) -> Result<Value, String> {
    let Some(items) = value.as_array() else {
        return accumulator.add_block(value);
    };
    items
        .iter()
        .map(|item| accumulator.add_block(item))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn skeletonize_messages(
    value: &Value,
    accumulator: &mut BlockAccumulator,
) -> Result<Value, String> {
    let Some(messages) = value.as_array() else {
        return accumulator.add_block(value);
    };
    messages
        .iter()
        .map(|message| skeletonize_message(message, accumulator))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn skeletonize_message(
    message: &Value,
    accumulator: &mut BlockAccumulator,
) -> Result<Value, String> {
    let Some(object) = message.as_object() else {
        return accumulator.add_block(message);
    };
    let Some(Value::Array(content)) = object.get("content") else {
        return accumulator.add_block(message);
    };
    let mut skeleton = object.clone();
    let content = content
        .iter()
        .map(|item| accumulator.add_block(item))
        .collect::<Result<Vec<_>, _>>()?;
    skeleton.insert("content".to_string(), Value::Array(content));
    Ok(Value::Object(skeleton))
}

fn skeletonize_input(value: &Value, accumulator: &mut BlockAccumulator) -> Result<Value, String> {
    if value.is_array() {
        skeletonize_array_items(value, accumulator)
    } else {
        accumulator.add_block(value)
    }
}

struct BlockAccumulator {
    trace_id: TraceId,
    action_id: String,
    refs: Vec<LlmRequestBlockRef>,
    blocks: BTreeMap<String, LlmRequestBlock>,
}

impl BlockAccumulator {
    fn new(trace_id: TraceId, action_id: &str) -> Self {
        Self {
            trace_id,
            action_id: action_id.to_string(),
            refs: Vec::new(),
            blocks: BTreeMap::new(),
        }
    }

    fn add_block(&mut self, value: &Value) -> Result<Value, String> {
        let ordinal = u32::try_from(self.refs.len())
            .map_err(|_| "LLM request block ordinal exceeds u32".to_string())?;
        let encoded_bytes = canonical_json_bytes(value);
        let block_hash = sha256_hex(&encoded_bytes);
        let block = LlmRequestBlock {
            trace_id: self.trace_id,
            block_hash: block_hash.clone(),
            uncompressed_bytes: encoded_bytes.len() as u64,
            encoded_bytes,
        };
        if let Some(existing) = self.blocks.get(&block_hash) {
            if existing != &block {
                return Err(format!(
                    "LLM request block hash collision for {}",
                    block_hash
                ));
            }
        } else {
            self.blocks.insert(block_hash.clone(), block);
        }
        self.refs.push(LlmRequestBlockRef {
            trace_id: self.trace_id,
            action_id: self.action_id.clone(),
            ordinal,
            block_hash,
        });
        Ok(block_placeholder(ordinal))
    }

    fn into_parts(self) -> (Vec<LlmRequestBlockRef>, Vec<LlmRequestBlock>) {
        (self.refs, self.blocks.into_values().collect())
    }
}

fn block_placeholder(ordinal: u32) -> Value {
    let mut object = Map::new();
    object.insert(
        BLOCK_PLACEHOLDER_KEY.to_string(),
        Value::Number(Number::from(ordinal)),
    );
    Value::Object(object)
}

fn canonical_json_bytes(value: &Value) -> Vec<u8> {
    canonical_json_string(value).into_bytes()
}

fn canonical_json_string(value: &Value) -> String {
    let mut output = String::new();
    write_canonical_json(&mut output, value);
    output
}

fn write_canonical_json(output: &mut String, value: &Value) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output
            .push_str(&serde_json::to_string(value).expect("serializing JSON string cannot fail")),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(output, value);
            }
            output.push(']');
        }
        Value::Object(object) => {
            output.push('{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("serializing JSON key cannot fail"),
                );
                output.push(':');
                write_canonical_json(output, &object[key]);
            }
            output.push('}');
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity("sha256:".len() + digest.len() * 2);
    output.push_str("sha256:");
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to string cannot fail");
    }
    output
}

fn message_preview(body: &Value) -> Option<String> {
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

fn user_message_metadata(body: &Value) -> UserMessageMetadata {
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
            .map(|message| sha256_hex(&canonical_json_bytes(message))),
    }
}

fn background_request_kind(body: &Value) -> Option<&'static str> {
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
