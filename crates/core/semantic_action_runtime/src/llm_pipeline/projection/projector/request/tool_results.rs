//! Provider-neutral projection of the tool results added since the model's latest turn.

use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use serde_json::Value;
use std::time::SystemTime;

use crate::llm_pipeline::projection::retention::canonical_llm_json_text;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectedLlmToolResult {
    pub(crate) trace_id: TraceId,
    pub(crate) process: ProcessIdentity,
    pub(crate) observed_at: SystemTime,
    pub(crate) request_action_id: String,
    pub(crate) tool_call_id: Option<String>,
    /// The tool call this result answers, as repeated in the same request.
    pub(crate) tool_call: Option<ProjectedLlmToolCall>,
    pub(crate) ordinal: usize,
    pub(crate) is_error: bool,
    pub(crate) content_json: Option<String>,
    pub(crate) content_bytes: Option<u64>,
    pub(crate) content_export_state: &'static str,
}

/// The tool call a result answers, as the agent repeats it in the request history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectedLlmToolCall {
    pub(crate) name: String,
}

pub(super) fn project_tool_results(
    config: &SemanticRetentionConfig,
    trace_id: TraceId,
    process: &ProcessIdentity,
    observed_at: SystemTime,
    request_action_id: &str,
    body: &Value,
) -> Vec<ProjectedLlmToolResult> {
    if !config.llm_layer_enabled() || !config.l0_llm_call.tool_results_enabled {
        return Vec::new();
    }
    let mut raw = Vec::new();
    let items = body
        .get("messages")
        .and_then(Value::as_array)
        .or_else(|| body.get("input").and_then(Value::as_array))
        .map_or(&[][..], Vec::as_slice);
    let turn_start = collect_current_turn_tool_results(items, &mut raw);
    let history = &items[..turn_start];
    raw.into_iter()
        .enumerate()
        .map(|(ordinal, raw)| {
            let tool_call = raw
                .tool_call_id
                .and_then(|id| declared_tool_call(history, id));
            let (content_json, content_bytes, content_export_state) =
                if config.llm_tool_result_content_export_enabled() {
                    let canonical_json = canonical_llm_json_text(raw.content);
                    let content_bytes = canonical_json.len() as u64;
                    if content_bytes <= config.l0_llm_call.tool_result_content_export_max_bytes {
                        (Some(canonical_json), Some(content_bytes), "exported")
                    } else {
                        (None, Some(content_bytes), "too_large")
                    }
                } else {
                    (None, None, "none")
                };
            ProjectedLlmToolResult {
                trace_id,
                process: *process,
                observed_at,
                request_action_id: request_action_id.to_string(),
                tool_call_id: raw.tool_call_id.map(ToString::to_string),
                tool_call,
                ordinal,
                is_error: raw.is_error,
                content_json,
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

/// Every request repeats the whole history, so only the tool results after the
/// model's latest output are new. Agents may append other messages after those
/// results (for example a `system` or `user` reminder), so the results do not
/// have to be at the end of the request.
///
/// Returns the index of the first item after the model's latest output.
fn collect_current_turn_tool_results<'a>(
    items: &'a [Value],
    output: &mut Vec<RawToolResult<'a>>,
) -> usize {
    let turn_start = items
        .iter()
        .rposition(is_model_output)
        .map_or(0, |index| index + 1);
    for item in &items[turn_start..] {
        collect_message_tool_results(item, output);
    }
    turn_start
}

/// Finds the call a result answers in the history that precedes the results.
/// Ids can repeat, so the nearest declaration wins.
fn declared_tool_call(history: &[Value], tool_call_id: &str) -> Option<ProjectedLlmToolCall> {
    history.iter().rev().find_map(|item| {
        item_tool_calls(item)
            .into_iter()
            .rev()
            .filter(|call| declares_tool_call_id(call, tool_call_id))
            .find_map(projected_tool_call)
    })
}

/// The tool calls one history item declares: a Responses API call item, the
/// `tool_calls` of an assistant message, or the call blocks of its content.
fn item_tool_calls(item: &Value) -> Vec<&Value> {
    if is_tool_call_kind(item) {
        return vec![item];
    }
    if item.get("role").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let declared = item.get("tool_calls").and_then(Value::as_array);
    let blocks = item
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| is_tool_call_kind(block));
    declared.into_iter().flatten().chain(blocks).collect()
}

fn is_tool_call_kind(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "tool_use" | "tool-call") || kind.ends_with("_call"))
}

fn declares_tool_call_id(call: &Value, tool_call_id: &str) -> bool {
    ["id", "call_id"]
        .into_iter()
        .any(|key| call.get(key).and_then(Value::as_str) == Some(tool_call_id))
}

/// Reads the tool name the same way the response projection does, so both
/// observations of one call report the same name.
fn projected_tool_call(call: &Value) -> Option<ProjectedLlmToolCall> {
    let name = call
        .pointer("/function/name")
        .and_then(Value::as_str)
        .or_else(|| call.get("name").and_then(Value::as_str))?;
    // The response projection qualifies Responses API MCP calls the same way.
    let name = call
        .get("namespace")
        .and_then(Value::as_str)
        .filter(|namespace| namespace.starts_with("mcp__") && !name.starts_with("mcp__"))
        .map_or_else(
            || name.to_string(),
            |namespace| format!("{namespace}__{name}"),
        );
    Some(ProjectedLlmToolCall { name })
}

/// An assistant message, or a Responses API call item such as `function_call`.
fn is_model_output(item: &Value) -> bool {
    item.get("role").and_then(Value::as_str) == Some("assistant")
        || item
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.ends_with("_call"))
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
