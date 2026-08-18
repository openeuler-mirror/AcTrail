//! Shared helpers for provider-specific LLM response parsing.

use semantic_action::{
    LlmParsedResponse, LlmParsedSseEvent, LlmTokenUsage, LlmToolCall, LlmToolFunction,
};
use serde_json::{Map, Number, Value};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ResponseTexts {
    pub(super) content_text: Option<String>,
    pub(super) reasoning_text: Option<String>,
}

impl ResponseTexts {
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

/// Reads token usage from a value that carries it under a `usage` key.
pub(in crate::payload_projection::llm) fn extract_token_usage(
    value: &Value,
) -> Option<LlmTokenUsage> {
    token_usage_from_map(value.get("usage")?.as_object()?)
}

/// The single source of truth for token usage field naming.
///
/// Providers spell the same counts differently and AcTrail cannot rely on
/// provider identification to pick a spelling: the request-side classifier
/// does not distinguish vendors, and a response may be parsed by whichever
/// registry entry matched. Every accepted spelling is therefore tried in one
/// place, so coverage does not depend on which parser handled the payload.
///
/// Nested detail objects take precedence over flat keys because the providers
/// that emit both document the nested form as authoritative.
pub(in crate::payload_projection::llm) fn token_usage_from_map(
    usage: &Map<String, Value>,
) -> Option<LlmTokenUsage> {
    let usage = LlmTokenUsage {
        // OpenAI chat/completions, then OpenAI Responses and Anthropic.
        prompt_tokens: token_count(usage.get("prompt_tokens"))
            .or_else(|| token_count(usage.get("input_tokens"))),
        completion_tokens: token_count(usage.get("completion_tokens"))
            .or_else(|| token_count(usage.get("output_tokens"))),
        // Anthropic reports no total.
        total_tokens: token_count(usage.get("total_tokens")),
        cached_prompt_tokens: token_count_from_details(
            usage,
            "prompt_tokens_details",
            "cached_tokens",
        )
        .or_else(|| token_count_from_details(usage, "input_tokens_details", "cached_tokens"))
        .or_else(|| token_count(usage.get("cache_read_input_tokens"))),
        reasoning_tokens: token_count_from_details(
            usage,
            "completion_tokens_details",
            "reasoning_tokens",
        )
        .or_else(|| token_count_from_details(usage, "output_tokens_details", "reasoning_tokens"))
        .or_else(|| token_count(usage.get("reasoning_tokens"))),
        // DeepSeek names a genuine hit count; Anthropic's cache read is the
        // same quantity under another name.
        prompt_cache_hit_tokens: token_count(usage.get("prompt_cache_hit_tokens"))
            .or_else(|| token_count(usage.get("cache_read_input_tokens"))),
        // Only a real miss count belongs here. Anthropic publishes no such
        // field, and its cache creation count is a different quantity.
        prompt_cache_miss_tokens: token_count(usage.get("prompt_cache_miss_tokens")),
        cache_creation_tokens: token_count(usage.get("cache_creation_input_tokens")),
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
        .or(usage.cache_creation_tokens)
        .is_some()
}

fn token_count(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn token_count_from_details(
    usage: &Map<String, Value>,
    details_key: &str,
    count_key: &str,
) -> Option<u64> {
    usage
        .get(details_key)
        .and_then(Value::as_object)
        .and_then(|details| token_count(details.get(count_key)))
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

#[derive(Clone, Default)]
pub(super) struct ToolCallAssembler {
    calls: Vec<LlmToolCall>,
}

#[derive(Clone, Copy)]
enum CodeModeLexState {
    Code,
    SingleQuoted,
    DoubleQuoted,
    Template,
    LineComment,
    BlockComment,
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
                match object.get("type").and_then(Value::as_str) {
                    Some("function_call") => self.apply_openai_response_function_call(object),
                    Some("custom_tool_call") => self.apply_openai_response_custom_tool_call(object),
                    _ => {}
                }
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

    fn apply_openai_response_function_call(&mut self, item: &Map<String, Value>) {
        let call_id = item.get("call_id").and_then(Value::as_str);
        let Some(call) = self.call_slot(None, call_id) else {
            return;
        };
        call.kind = Some("function".to_string());
        let function = call.function.get_or_insert_with(LlmToolFunction::default);
        if let Some(name) = Self::qualified_response_tool_name(item) {
            function.name = Some(name);
        }
        if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
            function.arguments = Some(arguments.to_string());
            function.arguments_json = parse_json_value(arguments);
        }
    }

    fn apply_openai_response_custom_tool_call(&mut self, item: &Map<String, Value>) {
        let raw_name = item
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty());
        let call_id = item.get("call_id").and_then(Value::as_str);
        if let Some(call) = self.call_slot(None, call_id) {
            call.kind = Some("custom".to_string());
            let function = call.function.get_or_insert_with(LlmToolFunction::default);
            function.name = Self::qualified_response_tool_name(item);
        }
        if raw_name == Some("exec")
            && let Some(input) = item.get("input").and_then(Value::as_str)
        {
            self.apply_code_mode_mcp_calls(input, call_id);
        }
    }

    fn apply_code_mode_mcp_calls(&mut self, input: &str, call_id: Option<&str>) {
        const PREFIX: &[u8] = b"tools.mcp__";

        let bytes = input.as_bytes();
        let mut cursor = 0;
        let mut state = CodeModeLexState::Code;
        while cursor < bytes.len() {
            match state {
                CodeModeLexState::Code => match bytes[cursor] {
                    b'\'' => {
                        state = CodeModeLexState::SingleQuoted;
                        cursor += 1;
                    }
                    b'"' => {
                        state = CodeModeLexState::DoubleQuoted;
                        cursor += 1;
                    }
                    b'`' => {
                        state = CodeModeLexState::Template;
                        cursor += 1;
                    }
                    b'/' if bytes.get(cursor + 1) == Some(&b'/') => {
                        state = CodeModeLexState::LineComment;
                        cursor += 2;
                    }
                    b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                        state = CodeModeLexState::BlockComment;
                        cursor += 2;
                    }
                    b'/' => break,
                    b't' if bytes[cursor..].starts_with(PREFIX) => {
                        let previous_significant = bytes[..cursor]
                            .iter()
                            .rfind(|byte| !byte.is_ascii_whitespace());
                        let preceded_by_identifier = cursor
                            .checked_sub(1)
                            .and_then(|index| bytes.get(index))
                            .is_some_and(|byte| {
                                byte.is_ascii_alphanumeric()
                                    || matches!(*byte, b'_' | b'$')
                                    || !byte.is_ascii()
                            })
                            || previous_significant == Some(&b'.');
                        let name_start = cursor + b"tools.".len();
                        let mut name_end = name_start;
                        while bytes
                            .get(name_end)
                            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                        {
                            name_end += 1;
                        }
                        cursor = name_end;
                        let mut call_start = name_end;
                        while bytes
                            .get(call_start)
                            .is_some_and(|byte| byte.is_ascii_whitespace())
                        {
                            call_start += 1;
                        }
                        if preceded_by_identifier || bytes.get(call_start) != Some(&b'(') {
                            continue;
                        }
                        let Ok(name) = std::str::from_utf8(&bytes[name_start..name_end]) else {
                            continue;
                        };
                        let valid_name = name.strip_prefix("mcp__").is_some_and(|encoded| {
                            encoded.split_once("__").is_some_and(|(server, tool)| {
                                !server.is_empty() && !tool.is_empty()
                            })
                        });
                        if !valid_name {
                            continue;
                        }
                        self.calls.push(LlmToolCall {
                            id: call_id.map(ToString::to_string),
                            kind: Some("function".to_string()),
                            function: Some(LlmToolFunction {
                                name: Some(name.to_string()),
                                ..LlmToolFunction::default()
                            }),
                            ..LlmToolCall::default()
                        });
                    }
                    _ => cursor += 1,
                },
                CodeModeLexState::SingleQuoted => {
                    if bytes[cursor] == b'\\' {
                        cursor = (cursor + 2).min(bytes.len());
                    } else {
                        if bytes[cursor] == b'\'' {
                            state = CodeModeLexState::Code;
                        }
                        cursor += 1;
                    }
                }
                CodeModeLexState::DoubleQuoted => {
                    if bytes[cursor] == b'\\' {
                        cursor = (cursor + 2).min(bytes.len());
                    } else {
                        if bytes[cursor] == b'"' {
                            state = CodeModeLexState::Code;
                        }
                        cursor += 1;
                    }
                }
                CodeModeLexState::Template => {
                    if bytes[cursor] == b'\\' {
                        cursor = (cursor + 2).min(bytes.len());
                    } else {
                        if bytes[cursor] == b'`' {
                            state = CodeModeLexState::Code;
                        }
                        cursor += 1;
                    }
                }
                CodeModeLexState::LineComment => {
                    if matches!(bytes[cursor], b'\n' | b'\r') {
                        state = CodeModeLexState::Code;
                    }
                    cursor += 1;
                }
                CodeModeLexState::BlockComment => {
                    if bytes[cursor] == b'*' && bytes.get(cursor + 1) == Some(&b'/') {
                        state = CodeModeLexState::Code;
                        cursor += 2;
                    } else {
                        cursor += 1;
                    }
                }
            }
        }
    }

    fn qualified_response_tool_name(item: &Map<String, Value>) -> Option<String> {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())?;
        Some(
            item.get("namespace")
                .and_then(Value::as_str)
                .filter(|namespace| namespace.starts_with("mcp__"))
                .filter(|_| !name.starts_with("mcp__"))
                .map(|namespace| format!("{namespace}__{name}"))
                .unwrap_or_else(|| name.to_string()),
        )
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

#[derive(Default)]
pub(super) struct ParsedSseResponseAccumulator {
    model: Option<String>,
    content_text: Option<String>,
    reasoning_text: Option<String>,
    tool_calls: ToolCallAssembler,
    chunk_count: usize,
    done: bool,
}

impl ParsedSseResponseAccumulator {
    pub(super) fn observe(&mut self, event: &LlmParsedSseEvent) {
        if self.model.is_none() {
            self.model = event.model.clone();
        }
        if let Some(content) = &event.content_text {
            append_text(&mut self.content_text, content);
            self.chunk_count += 1;
        }
        if let Some(reasoning) = &event.reasoning_text {
            append_text(&mut self.reasoning_text, reasoning);
            self.chunk_count += 1;
        }
        for tool_call in &event.tool_calls {
            self.tool_calls.apply_call_delta(tool_call.clone());
        }
        self.done |= event.done || event.finish_reason.is_some();
    }

    pub(super) fn finish(
        &self,
        provider_id: &'static str,
        token_usage: Option<LlmTokenUsage>,
        stream: bool,
    ) -> Option<LlmParsedResponse> {
        let tool_calls = self.tool_calls.clone().into_calls();
        if self.content_text.is_none()
            && self.reasoning_text.is_none()
            && tool_calls.is_empty()
            && !self.done
        {
            return None;
        }
        Some(LlmParsedResponse {
            provider_id,
            model: self.model.clone(),
            content_text: self.content_text.clone(),
            reasoning_text: self.reasoning_text.clone(),
            tool_calls,
            token_usage,
            chunk_count: self.chunk_count,
            done: self.done,
            stream,
        })
    }
}

fn append_text(target: &mut Option<String>, value: &str) {
    match target {
        Some(existing) => existing.push_str(value),
        None => *target = Some(value.to_string()),
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

#[cfg(test)]
mod tests {
    use super::extract_token_usage;
    use serde_json::json;

    /// Anthropic Messages reports cache accounting as flat top-level keys.
    /// The shared extractor only looked for the OpenAI nested shape, so these
    /// were silently dropped for every request sent to /v1/messages.
    #[test]
    fn reads_anthropic_flat_cache_fields() {
        let usage = extract_token_usage(&json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 800,
                "cache_creation_input_tokens": 200
            }
        }))
        .expect("anthropic usage is recognised");

        assert_eq!(usage.prompt_tokens, Some(100));
        assert_eq!(usage.completion_tokens, Some(50));
        assert_eq!(usage.cached_prompt_tokens, Some(800));
        assert_eq!(usage.prompt_cache_hit_tokens, Some(800));
        // Cache creation is not a cache miss: it is prompt content the
        // provider stored for reuse, billed separately. Anthropic has no
        // field meaning "billed at full rate because the cache did not
        // serve it", so miss stays empty rather than borrowing this value.
        assert_eq!(usage.cache_creation_tokens, Some(200));
        assert_eq!(usage.prompt_cache_miss_tokens, None);
        // Anthropic does not report a total, so the parsed usage stays
        // faithful to the wire.
        assert_eq!(usage.total_tokens, None);
    }

    #[test]
    fn reads_openai_nested_details() {
        let usage = extract_token_usage(&json!({
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30,
                "prompt_tokens_details": { "cached_tokens": 5 },
                "completion_tokens_details": { "reasoning_tokens": 7 }
            }
        }))
        .expect("openai usage is recognised");

        assert_eq!(usage.prompt_tokens, Some(10));
        assert_eq!(usage.completion_tokens, Some(20));
        assert_eq!(usage.total_tokens, Some(30));
        assert_eq!(usage.cached_prompt_tokens, Some(5));
        assert_eq!(usage.reasoning_tokens, Some(7));
    }

    #[test]
    fn reads_openai_responses_input_output_details() {
        let usage = extract_token_usage(&json!({
            "usage": {
                "input_tokens": 11,
                "output_tokens": 22,
                "total_tokens": 33,
                "input_tokens_details": { "cached_tokens": 6 },
                "output_tokens_details": { "reasoning_tokens": 8 }
            }
        }))
        .expect("openai responses usage is recognised");

        assert_eq!(usage.prompt_tokens, Some(11));
        assert_eq!(usage.completion_tokens, Some(22));
        assert_eq!(usage.cached_prompt_tokens, Some(6));
        assert_eq!(usage.reasoning_tokens, Some(8));
    }

    /// DeepSeek's OpenAI-compatible endpoint reports cache accounting under its
    /// own flat keys. This already worked and must keep working.
    #[test]
    fn reads_deepseek_flat_cache_fields() {
        let usage = extract_token_usage(&json!({
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30,
                "prompt_cache_hit_tokens": 8,
                "prompt_cache_miss_tokens": 2
            }
        }))
        .expect("deepseek usage is recognised");

        assert_eq!(usage.prompt_cache_hit_tokens, Some(8));
        assert_eq!(usage.prompt_cache_miss_tokens, Some(2));
        // DeepSeek reports a genuine miss count and never a creation count.
        assert_eq!(usage.cache_creation_tokens, None);
    }

    /// A nested OpenAI shape and a flat Anthropic shape must not fight: when
    /// both appear the nested detail wins, because that is the shape the
    /// provider that emits both actually documents.
    #[test]
    fn prefers_nested_details_when_both_shapes_present() {
        let usage = extract_token_usage(&json!({
            "usage": {
                "input_tokens": 1,
                "output_tokens": 2,
                "prompt_tokens_details": { "cached_tokens": 5 },
                "cache_read_input_tokens": 800
            }
        }))
        .expect("mixed usage is recognised");

        assert_eq!(usage.cached_prompt_tokens, Some(5));
    }

    #[test]
    fn returns_none_when_no_counts_present() {
        assert!(extract_token_usage(&json!({ "usage": {} })).is_none());
        assert!(extract_token_usage(&json!({})).is_none());
    }

    /// Every precedence chain is pinned, not just the cached-prompt one: a
    /// merge that quietly reorders any of them would otherwise pass.
    #[test]
    fn flat_keys_never_override_nested_details() {
        let usage = extract_token_usage(&json!({
            "usage": {
                "input_tokens": 1,
                "output_tokens": 2,
                "prompt_tokens_details": { "cached_tokens": 5 },
                "completion_tokens_details": { "reasoning_tokens": 9 },
                "cache_read_input_tokens": 800,
                "reasoning_tokens": 77
            }
        }))
        .expect("mixed usage is recognised");

        assert_eq!(usage.cached_prompt_tokens, Some(5));
        assert_eq!(usage.reasoning_tokens, Some(9));
    }

    /// DeepSeek's own hit count outranks Anthropic's cache read, which is the
    /// same quantity under a name only Anthropic uses.
    #[test]
    fn a_reported_hit_count_outranks_a_cache_read() {
        let usage = extract_token_usage(&json!({
            "usage": {
                "prompt_tokens": 10,
                "prompt_cache_hit_tokens": 8,
                "cache_read_input_tokens": 800
            }
        }))
        .expect("usage is recognised");

        assert_eq!(usage.prompt_cache_hit_tokens, Some(8));
    }

    /// Anthropic's cache read is the only signal it gives for cached prompt
    /// content, so it must reach both fields that describe that quantity.
    #[test]
    fn an_anthropic_cache_read_fills_both_cached_and_hit() {
        let usage = extract_token_usage(&json!({
            "usage": { "input_tokens": 1, "cache_read_input_tokens": 800 }
        }))
        .expect("usage is recognised");

        assert_eq!(usage.cached_prompt_tokens, Some(800));
        assert_eq!(usage.prompt_cache_hit_tokens, Some(800));
    }
}
