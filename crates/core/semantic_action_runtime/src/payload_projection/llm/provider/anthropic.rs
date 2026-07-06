//! Anthropic Messages response parser.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmProviderMatch,
    LlmProviderResponseParser, LlmProviderResponseStreamParser, LlmSseEvent, LlmSseResponseInput,
    LlmToolCall, LlmToolFunction,
};
use serde_json::Value;

use super::anthropic_stream::{AnthropicMessagesStreamParser, parsed_events_to_response};
use super::common::{
    ToolCallAssembler, extract_finish_reason, extract_token_usage, extract_token_usage_from_values,
    parse_json_value, push_non_empty_text,
};

pub(super) const ANTHROPIC_PROVIDER_ID: &str = "anthropic-messages";

const MESSAGE_TYPE: &str = "message";
const MESSAGE_START: &str = "message_start";
const MESSAGE_DELTA: &str = "message_delta";
const MESSAGE_STOP: &str = "message_stop";
const CONTENT_BLOCK_START: &str = "content_block_start";
const CONTENT_BLOCK_DELTA: &str = "content_block_delta";
const CONTENT_BLOCK_STOP: &str = "content_block_stop";
const CONTENT_TEXT: &str = "text";
const CONTENT_THINKING: &str = "thinking";
const CONTENT_TOOL_USE: &str = "tool_use";
const DELTA_TEXT: &str = "text_delta";
const DELTA_THINKING: &str = "thinking_delta";
const DELTA_INPUT_JSON: &str = "input_json_delta";
const TOOL_CALL_KIND_FUNCTION: &str = "function";

#[derive(Default)]
pub(super) struct AnthropicMessagesResponseParser;

impl LlmProviderResponseParser for AnthropicMessagesResponseParser {
    fn provider_id(&self) -> &'static str {
        ANTHROPIC_PROVIDER_ID
    }

    fn match_json_response(&self, input: LlmJsonResponseInput<'_>) -> LlmProviderMatch {
        if anthropic_json_message(input.json) {
            LlmProviderMatch::Strong
        } else {
            LlmProviderMatch::NoMatch
        }
    }

    fn match_sse_response(&self, input: LlmSseResponseInput<'_>) -> LlmProviderMatch {
        if input
            .events
            .iter()
            .filter_map(|event| event.json)
            .any(anthropic_sse_event)
        {
            LlmProviderMatch::Strong
        } else {
            LlmProviderMatch::NoMatch
        }
    }

    fn parse_json_response(&self, input: LlmJsonResponseInput<'_>) -> Option<LlmParsedResponse> {
        if self.match_json_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let mut content_chunks = Vec::new();
        let mut reasoning_chunks = Vec::new();
        let mut assembler = ToolCallAssembler::default();
        if let Some(content) = input.json.get("content").and_then(Value::as_array) {
            for (index, item) in content.iter().enumerate() {
                collect_json_content_item(
                    item,
                    index,
                    &mut content_chunks,
                    &mut reasoning_chunks,
                    &mut assembler,
                );
            }
        }
        let tool_calls = assembler.into_calls();
        if content_chunks.is_empty() && reasoning_chunks.is_empty() && tool_calls.is_empty() {
            return None;
        }
        let content_text = (!content_chunks.is_empty()).then(|| content_chunks.join(""));
        let reasoning_text = (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join(""));
        Some(LlmParsedResponse {
            provider_id: self.provider_id(),
            model: input
                .json
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content_text,
            reasoning_text,
            tool_calls,
            token_usage: extract_token_usage(input.json),
            chunk_count: content_chunks.len() + reasoning_chunks.len(),
            done: input.json.get("stop_reason").is_some(),
            stream: false,
        })
    }

    fn parse_sse_response(&self, input: LlmSseResponseInput<'_>) -> Option<LlmParsedResponse> {
        if self.match_sse_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let parsed_events = input
            .events
            .iter()
            .map(|event| self.parse_sse_event(*event))
            .collect::<Vec<_>>();
        parsed_events_to_response(
            self.provider_id(),
            &parsed_events,
            extract_token_usage_from_values(input.events.iter().filter_map(|event| event.json)),
            true,
        )
    }

    fn parse_sse_event(&self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        let Some(value) = event.json else {
            return LlmParsedSseEvent {
                done: event.done_marker,
                ..LlmParsedSseEvent::default()
            };
        };
        match value.get("type").and_then(Value::as_str) {
            Some(MESSAGE_START) => message_start_event(value),
            Some(CONTENT_BLOCK_START) => content_block_start_event(value),
            Some(CONTENT_BLOCK_DELTA) => content_block_delta_event(value),
            Some(MESSAGE_DELTA) => message_delta_event(value),
            Some(MESSAGE_STOP) => LlmParsedSseEvent {
                done: true,
                ..LlmParsedSseEvent::default()
            },
            _ => LlmParsedSseEvent::default(),
        }
    }

    fn new_stream_parser(&self) -> Box<dyn LlmProviderResponseStreamParser + Send> {
        Box::new(AnthropicMessagesStreamParser::default())
    }
}

fn anthropic_json_message(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.get("type").and_then(Value::as_str) == Some(MESSAGE_TYPE)
        && object.contains_key("model")
        && object.contains_key("content")
}

fn anthropic_sse_event(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some(MESSAGE_START)
            | Some(MESSAGE_DELTA)
            | Some(MESSAGE_STOP)
            | Some(CONTENT_BLOCK_START)
            | Some(CONTENT_BLOCK_DELTA)
            | Some(CONTENT_BLOCK_STOP)
    )
}

fn message_start_event(value: &Value) -> LlmParsedSseEvent {
    LlmParsedSseEvent {
        model: value
            .get("message")
            .and_then(|message| message.get("model"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        ..LlmParsedSseEvent::default()
    }
}

fn content_block_start_event(value: &Value) -> LlmParsedSseEvent {
    let Some(block) = value.get("content_block") else {
        return LlmParsedSseEvent::default();
    };
    match block.get("type").and_then(Value::as_str) {
        Some(CONTENT_TEXT) => LlmParsedSseEvent {
            content_text: block
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string),
            ..LlmParsedSseEvent::default()
        },
        Some(CONTENT_THINKING) => LlmParsedSseEvent {
            reasoning_text: block
                .get("thinking")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string),
            ..LlmParsedSseEvent::default()
        },
        Some(CONTENT_TOOL_USE) => LlmParsedSseEvent {
            tool_calls: tool_use_start_call(value, block, false)
                .into_iter()
                .collect(),
            ..LlmParsedSseEvent::default()
        },
        _ => LlmParsedSseEvent::default(),
    }
}

fn content_block_delta_event(value: &Value) -> LlmParsedSseEvent {
    let Some(delta) = value.get("delta") else {
        return LlmParsedSseEvent::default();
    };
    match delta.get("type").and_then(Value::as_str) {
        Some(DELTA_TEXT) => LlmParsedSseEvent {
            content_text: delta
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string),
            ..LlmParsedSseEvent::default()
        },
        Some(DELTA_THINKING) => LlmParsedSseEvent {
            reasoning_text: delta
                .get("thinking")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string),
            ..LlmParsedSseEvent::default()
        },
        Some(DELTA_INPUT_JSON) => LlmParsedSseEvent {
            tool_calls: input_json_delta_call(value, delta).into_iter().collect(),
            ..LlmParsedSseEvent::default()
        },
        _ => LlmParsedSseEvent::default(),
    }
}

fn message_delta_event(value: &Value) -> LlmParsedSseEvent {
    let finish_reason = value.get("delta").and_then(extract_finish_reason);
    LlmParsedSseEvent {
        done: finish_reason.is_some(),
        finish_reason,
        ..LlmParsedSseEvent::default()
    }
}

fn collect_json_content_item(
    item: &Value,
    index: usize,
    content_chunks: &mut Vec<String>,
    reasoning_chunks: &mut Vec<String>,
    assembler: &mut ToolCallAssembler,
) {
    match item.get("type").and_then(Value::as_str) {
        Some(CONTENT_TEXT) => {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                push_non_empty_text(content_chunks, text);
            }
        }
        Some(CONTENT_THINKING) => {
            if let Some(text) = item.get("thinking").and_then(Value::as_str) {
                push_non_empty_text(reasoning_chunks, text);
            }
        }
        Some(CONTENT_TOOL_USE) => {
            if let Some(call) = json_tool_use_call(item, index) {
                assembler.apply_call_delta(call);
            }
        }
        _ => {}
    }
}

fn json_tool_use_call(item: &Value, index: usize) -> Option<LlmToolCall> {
    let index = u64::try_from(index).ok()?;
    let function = LlmToolFunction {
        name: item
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string),
        arguments: item.get("input").map(Value::to_string),
        arguments_json: item.get("input").cloned(),
    };
    Some(LlmToolCall {
        index: Some(index),
        id: item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string),
        kind: Some(TOOL_CALL_KIND_FUNCTION.to_string()),
        function: Some(function),
    })
}

fn tool_use_start_call(
    value: &Value,
    block: &Value,
    include_empty_input: bool,
) -> Option<LlmToolCall> {
    let function = LlmToolFunction {
        name: block
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string),
        arguments: block
            .get("input")
            .filter(|input| include_empty_input || !empty_json_object(input))
            .map(Value::to_string),
        arguments_json: block
            .get("input")
            .filter(|input| include_empty_input || !empty_json_object(input))
            .cloned(),
    };
    Some(LlmToolCall {
        index: value.get("index").and_then(Value::as_u64),
        id: block
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string),
        kind: Some(TOOL_CALL_KIND_FUNCTION.to_string()),
        function: Some(function),
    })
}

fn input_json_delta_call(value: &Value, delta: &Value) -> Option<LlmToolCall> {
    let partial = delta.get("partial_json").and_then(Value::as_str)?;
    Some(LlmToolCall {
        index: value.get("index").and_then(Value::as_u64),
        function: Some(LlmToolFunction {
            arguments: Some(partial.to_string()),
            arguments_json: parse_json_value(partial),
            ..LlmToolFunction::default()
        }),
        ..LlmToolCall::default()
    })
}

fn empty_json_object(value: &Value) -> bool {
    value.as_object().is_some_and(|object| object.is_empty())
}
