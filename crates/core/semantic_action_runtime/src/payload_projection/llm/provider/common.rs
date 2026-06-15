//! Shared helpers for provider-specific LLM response parsing.

use semantic_action::{LlmTokenUsage, LlmToolCall, LlmToolFunction};
use serde_json::{Map, Number, Value};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ResponseTexts {
    pub(super) content_text: Option<String>,
    pub(super) reasoning_text: Option<String>,
}

impl ResponseTexts {
    pub(super) fn output_text(&self) -> Option<String> {
        self.content_text
            .clone()
            .or_else(|| self.reasoning_text.clone())
    }

    pub(super) fn chunk_count(&self) -> usize {
        usize::from(self.content_text.is_some()) + usize::from(self.reasoning_text.is_some())
    }
}

pub(super) fn extract_response_texts(value: &Value) -> ResponseTexts {
    let mut content_chunks = Vec::new();
    let mut reasoning_chunks = Vec::new();
    collect_response_text(value, &mut content_chunks, &mut reasoning_chunks);
    ResponseTexts {
        content_text: (!content_chunks.is_empty()).then(|| content_chunks.join("")),
        reasoning_text: (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join("")),
    }
}

fn collect_response_text(
    value: &Value,
    content_chunks: &mut Vec<String>,
    reasoning_chunks: &mut Vec<String>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_response_text(item, content_chunks, reasoning_chunks);
            }
        }
        Value::Object(object) => {
            for key in ["content", "text", "output_text"] {
                if let Some(text) = object.get(key).and_then(Value::as_str) {
                    push_non_empty_text(content_chunks, text);
                }
            }
            if let Some(text) = object.get("reasoning_content").and_then(Value::as_str) {
                push_non_empty_text(reasoning_chunks, text);
            }
            if let Some(text) = object.get("thinking").and_then(Value::as_str) {
                push_non_empty_text(reasoning_chunks, text);
            }
            for key in ["content", "message", "delta", "choices", "output"] {
                if let Some(child) = object.get(key) {
                    collect_response_text(child, content_chunks, reasoning_chunks);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn push_non_empty_text(chunks: &mut Vec<String>, text: &str) {
    if !text.is_empty() {
        chunks.push(text.to_string());
    }
}

pub(super) fn extract_token_usage(value: &Value) -> Option<LlmTokenUsage> {
    let usage = value.get("usage")?.as_object()?;
    let prompt_tokens =
        token_count(usage.get("prompt_tokens")).or_else(|| token_count(usage.get("input_tokens")));
    let completion_tokens = token_count(usage.get("completion_tokens"))
        .or_else(|| token_count(usage.get("output_tokens")));
    let total_tokens = token_count(usage.get("total_tokens"));
    let cached_prompt_tokens = usage
        .get("prompt_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| token_count(details.get("cached_tokens")));
    let reasoning_tokens = usage
        .get("completion_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| token_count(details.get("reasoning_tokens")));
    let prompt_cache_hit_tokens = token_count(usage.get("prompt_cache_hit_tokens"));
    let prompt_cache_miss_tokens = token_count(usage.get("prompt_cache_miss_tokens"));
    let usage = LlmTokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_prompt_tokens,
        reasoning_tokens,
        prompt_cache_hit_tokens,
        prompt_cache_miss_tokens,
    };
    token_usage_has_any_count(&usage).then_some(usage)
}

pub(super) fn extract_token_usage_from_values<'a>(
    values: impl IntoIterator<Item = &'a Value>,
) -> Option<LlmTokenUsage> {
    values.into_iter().filter_map(extract_token_usage).last()
}

fn token_usage_has_any_count(usage: &LlmTokenUsage) -> bool {
    usage
        .prompt_tokens
        .or(usage.completion_tokens)
        .or(usage.total_tokens)
        .or(usage.cached_prompt_tokens)
        .or(usage.reasoning_tokens)
        .or(usage.prompt_cache_hit_tokens)
        .or(usage.prompt_cache_miss_tokens)
        .is_some()
}

fn token_count(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

pub(super) fn extract_finish_reason(value: &Value) -> Option<String> {
    match value {
        Value::Array(items) => items.iter().find_map(extract_finish_reason),
        Value::Object(object) => object
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| {
                object
                    .get("stop_reason")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .or_else(|| object.get("choices").and_then(extract_finish_reason))
            .or_else(|| object.get("delta").and_then(extract_finish_reason)),
        _ => None,
    }
}

pub(in crate::payload_projection::llm) fn tool_calls_json(
    tool_calls: &[LlmToolCall],
) -> Option<String> {
    let values = tool_calls
        .iter()
        .filter_map(tool_call_value)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| Value::Array(values).to_string())
}

fn tool_call_value(tool_call: &LlmToolCall) -> Option<Value> {
    let mut object = Map::new();
    if let Some(index) = tool_call.index {
        object.insert("index".to_string(), Value::Number(Number::from(index)));
    }
    if let Some(id) = &tool_call.id {
        object.insert("id".to_string(), Value::String(id.clone()));
    }
    if let Some(kind) = &tool_call.kind {
        object.insert("type".to_string(), Value::String(kind.clone()));
    }
    if let Some(function) = tool_call.function.as_ref().and_then(tool_function_value) {
        object.insert("function".to_string(), function);
    }
    (!object.is_empty()).then(|| Value::Object(object))
}

fn tool_function_value(function: &LlmToolFunction) -> Option<Value> {
    let mut object = Map::new();
    if let Some(name) = &function.name {
        object.insert("name".to_string(), Value::String(name.clone()));
    }
    if let Some(arguments) = &function.arguments {
        object.insert("arguments".to_string(), Value::String(arguments.clone()));
    }
    if let Some(arguments_json) = &function.arguments_json {
        object.insert("arguments_json".to_string(), arguments_json.clone());
    }
    (!object.is_empty()).then(|| Value::Object(object))
}

#[derive(Default)]
pub(super) struct ToolCallAssembler {
    calls: Vec<LlmToolCall>,
}

impl ToolCallAssembler {
    pub(super) fn apply_values<'a>(&mut self, values: impl IntoIterator<Item = &'a Value>) {
        for value in values {
            self.apply_value(value);
        }
    }

    pub(super) fn apply_value(&mut self, value: &Value) {
        match value {
            Value::Array(items) => {
                for item in items {
                    self.apply_value(item);
                }
            }
            Value::Object(object) => {
                if let Some(Value::Array(tool_calls)) = object.get("tool_calls") {
                    for tool_call in tool_calls {
                        if let Value::Object(tool_call) = tool_call {
                            self.apply_openai_delta(tool_call);
                        }
                    }
                }
                for key in ["message", "delta", "choices", "output"] {
                    if let Some(child) = object.get(key) {
                        self.apply_value(child);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn apply_call_delta(&mut self, delta: LlmToolCall) {
        let Some(call) = self.call_slot(delta.index, delta.id.as_deref()) else {
            return;
        };
        if let Some(index) = delta.index {
            call.index.get_or_insert(index);
        }
        if let Some(id) = delta.id.filter(|id| !id.is_empty()) {
            call.id.get_or_insert(id);
        }
        if let Some(kind) = delta.kind.filter(|kind| !kind.is_empty()) {
            call.kind = Some(kind);
        }
        if let Some(function) = delta.function {
            apply_function_delta(call, function);
        }
    }

    pub(super) fn into_calls(self) -> Vec<LlmToolCall> {
        self.calls
            .into_iter()
            .filter(|call| tool_call_value(call).is_some())
            .collect()
    }

    fn apply_openai_delta(&mut self, delta: &Map<String, Value>) {
        let index = delta.get("index").and_then(Value::as_u64);
        let id = delta.get("id").and_then(Value::as_str);
        let Some(call) = self.call_slot(index, id) else {
            return;
        };
        if let Some(index) = index {
            call.index.get_or_insert(index);
        }
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            call.id.get_or_insert_with(|| id.to_string());
        }
        if let Some(kind) = delta
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty())
        {
            call.kind = Some(kind.to_string());
        }
        if let Some(function) = delta.get("function").and_then(Value::as_object) {
            let function = LlmToolFunction {
                name: function
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .map(ToString::to_string),
                arguments: function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                arguments_json: function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(parse_json_value),
            };
            apply_function_delta(call, function);
        }
    }

    fn call_slot(&mut self, index: Option<u64>, id: Option<&str>) -> Option<&mut LlmToolCall> {
        let has_id = id.filter(|id| !id.is_empty()).is_some();
        if index.is_none() && !has_id {
            return None;
        }
        if let Some(position) = self.calls.iter().position(|call| {
            index.is_some_and(|index| call.index == Some(index))
                || id
                    .filter(|id| !id.is_empty())
                    .is_some_and(|id| call.id.as_deref() == Some(id))
        }) {
            return self.calls.get_mut(position);
        }
        self.calls.push(LlmToolCall {
            index,
            id: id.filter(|id| !id.is_empty()).map(ToString::to_string),
            ..LlmToolCall::default()
        });
        self.calls.last_mut()
    }
}

fn apply_function_delta(call: &mut LlmToolCall, delta: LlmToolFunction) {
    let function = call.function.get_or_insert_with(LlmToolFunction::default);
    if let Some(name) = delta.name.filter(|name| !name.is_empty()) {
        function.name = Some(name);
    }
    if let Some(arguments) = delta.arguments {
        match &mut function.arguments {
            Some(existing) => existing.push_str(&arguments),
            None => function.arguments = Some(arguments),
        }
        function.arguments_json = function.arguments.as_deref().and_then(parse_json_value);
    }
    if delta.arguments_json.is_some() {
        function.arguments_json = delta.arguments_json;
    }
}

pub(super) fn parse_json_value(text: &str) -> Option<Value> {
    serde_json::from_str::<Value>(text).ok()
}
