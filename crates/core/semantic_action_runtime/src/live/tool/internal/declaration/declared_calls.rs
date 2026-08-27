//! Decoding of tool calls declared by an LLM response.

use semantic_action::{SemanticAction, attr_keys as attrs};
use serde_json::Value;

use crate::llm_pipeline::canonical_llm_json;

#[derive(Clone, Debug)]
pub(in crate::live::tool) struct DeclaredLlmToolCall {
    pub(in crate::live::tool) ordinal: usize,
    pub(in crate::live::tool) tool_call_id: Option<String>,
    pub(in crate::live::tool) name: String,
    pub(in crate::live::tool) arguments: Value,
    pub(in crate::live::tool) arguments_json: String,
    pub(in crate::live::tool) arguments_hash: String,
}

#[derive(Default)]
pub(in crate::live::tool) struct DeclaredLlmToolCalls {
    pub(in crate::live::tool) calls: Vec<DeclaredLlmToolCall>,
    pub(in crate::live::tool) malformed: bool,
    pub(in crate::live::tool) discarded_entries: u64,
}

impl DeclaredLlmToolCalls {
    pub(in crate::live::tool) fn from_response(action: &SemanticAction) -> Self {
        let Some(tool_calls_json) = action.attributes.get(attrs::llm_response::TOOL_CALLS_JSON)
        else {
            return Self::default();
        };
        let Ok(Value::Array(tool_calls)) = serde_json::from_str::<Value>(tool_calls_json) else {
            return Self {
                malformed: true,
                discarded_entries: 1,
                ..Self::default()
            };
        };
        let mut parsed = Self::default();
        for (ordinal, tool_call) in tool_calls.into_iter().enumerate() {
            let Some(name) = tool_call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .or_else(|| tool_call.get("name").and_then(Value::as_str))
            else {
                parsed.discarded_entries = parsed.discarded_entries.saturating_add(1);
                continue;
            };
            let arguments = tool_call
                .pointer("/function/arguments")
                .or_else(|| tool_call.get("arguments"))
                .or_else(|| tool_call.get("input"))
                .map(Self::parse_arguments)
                .unwrap_or(Value::Null);
            let (arguments_json, arguments_hash) = canonical_llm_json(&arguments);
            parsed.calls.push(DeclaredLlmToolCall {
                ordinal,
                tool_call_id: tool_call
                    .get("id")
                    .or_else(|| tool_call.get("call_id"))
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(ToString::to_string),
                name: name.to_string(),
                arguments,
                arguments_json,
                arguments_hash,
            });
        }
        parsed
    }

    fn parse_arguments(value: &Value) -> Value {
        value
            .as_str()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_else(|| value.clone())
    }
}
