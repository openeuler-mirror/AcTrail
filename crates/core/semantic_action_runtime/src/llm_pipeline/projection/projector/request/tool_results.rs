//! Provider-neutral projection of the current request's trailing tool results.

use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use serde_json::Value;
use std::time::SystemTime;

use crate::llm_pipeline::projection::retention::canonical_llm_json;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectedLlmToolResult {
    pub(crate) trace_id: TraceId,
    pub(crate) process: ProcessIdentity,
    pub(crate) observed_at: SystemTime,
    pub(crate) request_action_id: String,
    pub(crate) tool_call_id: Option<String>,
    pub(crate) ordinal: usize,
    pub(crate) is_error: bool,
    pub(crate) content_json: Option<String>,
    pub(crate) content_hash: String,
    pub(crate) content_bytes: u64,
    pub(crate) content_export_state: &'static str,
}

pub(super) fn project_tool_results(
    config: &SemanticRetentionConfig,
    trace_id: TraceId,
    process: &ProcessIdentity,
    observed_at: SystemTime,
    request_action_id: &str,
    body: &Value,
) -> Vec<ProjectedLlmToolResult> {
    if !config.llm_layer_enabled() {
        return Vec::new();
    }
    let mut raw = Vec::new();
    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        collect_trailing_tool_results(messages, &mut raw);
    } else if let Some(input) = body.get("input").and_then(Value::as_array) {
        collect_trailing_tool_results(input, &mut raw);
    }
    raw.into_iter()
        .enumerate()
        .map(|(ordinal, raw)| {
            let (canonical_json, content_hash) = canonical_llm_json(raw.content);
            let content_bytes = canonical_json.len() as u64;
            let content_json = if !config.llm_tool_result_content_export_enabled() {
                None
            } else if content_bytes <= config.l0_llm_call.tool_result_content_export_max_bytes {
                Some(canonical_json)
            } else {
                None
            };
            let content_export_state = if !config.llm_tool_result_content_export_enabled() {
                "none"
            } else if content_json.is_some() {
                "exported"
            } else {
                "too_large"
            };
            ProjectedLlmToolResult {
                trace_id,
                process: *process,
                observed_at,
                request_action_id: request_action_id.to_string(),
                tool_call_id: raw.tool_call_id.map(ToString::to_string),
                ordinal,
                is_error: raw.is_error,
                content_json,
                content_hash,
                content_bytes,
                content_export_state,
            }
        })
        .collect()
}

struct RawToolResult<'a> {
    tool_call_id: Option<&'a str>,
    is_error: bool,
    content: &'a Value,
}

fn collect_trailing_tool_results<'a>(items: &'a [Value], output: &mut Vec<RawToolResult<'a>>) {
    let mut suffix = Vec::new();
    for item in items.iter().rev() {
        let mut item_results = Vec::new();
        collect_message_tool_results(item, &mut item_results);
        if item_results.is_empty() {
            break;
        }
        suffix.push(item_results);
    }
    for item_results in suffix.into_iter().rev() {
        output.extend(item_results);
    }
}

fn collect_message_tool_results<'a>(message: &'a Value, output: &mut Vec<RawToolResult<'a>>) {
    let kind = message.get("type").and_then(Value::as_str);
    let role = message.get("role").and_then(Value::as_str);
    if kind.is_some_and(is_tool_result_kind) || role == Some("tool") {
        output.push(raw_tool_result(message));
        return;
    }
    let Some(content) = message.get("content") else {
        return;
    };
    match content {
        Value::Array(blocks) => {
            for block in blocks {
                if block
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(is_tool_result_kind)
                {
                    output.push(raw_tool_result(block));
                }
            }
        }
        Value::Object(_)
            if content
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(is_tool_result_kind) =>
        {
            output.push(raw_tool_result(content));
        }
        _ => {}
    }
}

fn is_tool_result_kind(kind: &str) -> bool {
    matches!(kind, "tool_result" | "tool-result") || kind.ends_with("_call_output")
}

fn raw_tool_result(value: &Value) -> RawToolResult<'_> {
    let content = value
        .get("content")
        .or_else(|| value.get("output"))
        .unwrap_or(&Value::Null);
    let tool_call_id = ["tool_use_id", "tool_call_id", "call_id", "id"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .filter(|id| !id.is_empty());
    let is_error = value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value.get("status").and_then(Value::as_str) == Some("error");
    RawToolResult {
        tool_call_id,
        is_error,
        content,
    }
}
