use std::collections::HashMap;

use serde_json::Value;

use crate::capture::SseFrame;

use super::super::model::{
    DEFAULT_STREAM_INDEX, JSON_FIELD_CONTENT, JSON_FIELD_TEXT, LlmAccumulator, LlmKey,
    LlmMessageStatus, LlmOutput, delta, message, text_field,
};

const JSON_FIELD_CONTENT_BLOCK: &str = "content_block";
const JSON_FIELD_DELTA: &str = "delta";
const JSON_FIELD_INDEX: &str = "index";
const JSON_FIELD_ROLE: &str = "role";
const JSON_FIELD_STOP_REASON: &str = "stop_reason";
const JSON_FIELD_TYPE: &str = "type";
const JSON_ROLE_ASSISTANT: &str = "assistant";
const JSON_TYPE_MESSAGE: &str = "message";
const JSON_TYPE_TEXT: &str = "text";

const EVENT_CONTENT_BLOCK_DELTA: &str = "content_block_delta";
const EVENT_CONTENT_BLOCK_START: &str = "content_block_start";
const EVENT_CONTENT_BLOCK_STOP: &str = "content_block_stop";
const EVENT_MESSAGE_DELTA: &str = "message_delta";
const EVENT_MESSAGE_START: &str = "message_start";
const EVENT_MESSAGE_STOP: &str = "message_stop";

#[derive(Debug, Default)]
pub(in crate::llm_projection) struct AnthropicMessagesParser {
    streams: HashMap<LlmKey, LlmAccumulator>,
}

impl AnthropicMessagesParser {
    pub(in crate::llm_projection) fn matches_frame(frame: &SseFrame, value: &Value) -> bool {
        matches!(
            event_type(frame, value),
            Some(EVENT_CONTENT_BLOCK_DELTA)
                | Some(EVENT_CONTENT_BLOCK_START)
                | Some(EVENT_CONTENT_BLOCK_STOP)
                | Some(EVENT_MESSAGE_DELTA)
                | Some(EVENT_MESSAGE_START)
                | Some(EVENT_MESSAGE_STOP)
        )
    }

    pub(in crate::llm_projection) fn matches_http(value: &Value) -> bool {
        value
            .get(JSON_FIELD_TYPE)
            .and_then(Value::as_str)
            .is_some_and(|value| value == JSON_TYPE_MESSAGE)
            && value
                .get(JSON_FIELD_ROLE)
                .and_then(Value::as_str)
                .map_or(true, |role| role == JSON_ROLE_ASSISTANT)
            && !message_blocks(value).is_empty()
    }

    pub(in crate::llm_projection) fn push_frame(
        &mut self,
        frame: &SseFrame,
        value: &Value,
    ) -> Vec<LlmOutput> {
        match event_type(frame, value) {
            Some(EVENT_CONTENT_BLOCK_DELTA) => self.push_delta(frame, value),
            Some(EVENT_CONTENT_BLOCK_START) => self.push_content_block_start(frame, value),
            Some(EVENT_CONTENT_BLOCK_STOP) => self.finish_content_block(frame, value),
            Some(EVENT_MESSAGE_STOP) => self.finish_stream(frame),
            Some(EVENT_MESSAGE_DELTA) => self.finish_if_stop_reason(frame, value),
            Some(EVENT_MESSAGE_START) => Vec::new(),
            _ => Vec::new(),
        }
    }

    pub(in crate::llm_projection) fn push_http_message(
        &mut self,
        key: LlmKey,
        value: &Value,
    ) -> Vec<LlmOutput> {
        let blocks = message_blocks(value);
        let reason = text_field(value, JSON_FIELD_STOP_REASON).map(ToString::to_string);
        blocks
            .into_iter()
            .map(|(content_index, text)| {
                message(
                    LlmKey::from_indices(
                        key.pid,
                        key.stream_key,
                        key.direction,
                        key.output_index,
                        content_index,
                    ),
                    LlmMessageStatus::Complete,
                    text,
                    reason.clone(),
                )
            })
            .collect()
    }

    pub(in crate::llm_projection) fn finish(&mut self) -> Vec<LlmOutput> {
        let keys = self.streams.keys().copied().collect::<Vec<_>>();
        let mut output = Vec::new();
        for key in keys {
            let Some(accumulator) = self.streams.remove(&key) else {
                continue;
            };
            if accumulator.text.is_empty() || accumulator.emitted_message {
                continue;
            }
            output.push(message(
                key,
                LlmMessageStatus::Partial,
                accumulator.text,
                Some("target exited before Anthropic message_stop".to_string()),
            ));
        }
        output
    }

    fn push_delta(&mut self, frame: &SseFrame, value: &Value) -> Vec<LlmOutput> {
        let Some(delta_text) = value
            .get(JSON_FIELD_DELTA)
            .and_then(|delta| text_field(delta, JSON_FIELD_TEXT))
            .filter(|text| !text.is_empty())
        else {
            return Vec::new();
        };
        let key = key_from_frame(frame, value);
        self.streams
            .entry(key)
            .or_default()
            .text
            .push_str(delta_text);
        vec![delta(key, delta_text)]
    }

    fn push_content_block_start(&mut self, frame: &SseFrame, value: &Value) -> Vec<LlmOutput> {
        let Some(text) = value
            .get(JSON_FIELD_CONTENT_BLOCK)
            .and_then(|block| text_field(block, JSON_FIELD_TEXT))
            .filter(|text| !text.is_empty())
        else {
            return Vec::new();
        };
        let key = key_from_frame(frame, value);
        self.streams.entry(key).or_default().text.push_str(text);
        vec![delta(key, text)]
    }

    fn finish_content_block(&mut self, frame: &SseFrame, value: &Value) -> Vec<LlmOutput> {
        self.finish_key(key_from_frame(frame, value), None)
    }

    fn finish_if_stop_reason(&mut self, frame: &SseFrame, value: &Value) -> Vec<LlmOutput> {
        let reason = value
            .get(JSON_FIELD_DELTA)
            .and_then(|delta| text_field(delta, JSON_FIELD_STOP_REASON))
            .map(ToString::to_string);
        if reason.is_none() {
            return Vec::new();
        }
        self.finish_stream_with_reason(frame, reason)
    }

    fn finish_stream(&mut self, frame: &SseFrame) -> Vec<LlmOutput> {
        self.finish_stream_with_reason(frame, None)
    }

    fn finish_stream_with_reason(
        &mut self,
        frame: &SseFrame,
        reason: Option<String>,
    ) -> Vec<LlmOutput> {
        let keys = self
            .streams
            .keys()
            .copied()
            .filter(|key| {
                key.pid == frame.pid
                    && key.stream_key == frame.stream_key
                    && key.direction == frame.direction
            })
            .collect::<Vec<_>>();
        keys.into_iter()
            .flat_map(|key| self.finish_key(key, reason.clone()))
            .collect()
    }

    fn finish_key(&mut self, key: LlmKey, reason: Option<String>) -> Vec<LlmOutput> {
        let Some(accumulator) = self.streams.get_mut(&key) else {
            return Vec::new();
        };
        if accumulator.text.is_empty() || accumulator.emitted_message {
            return Vec::new();
        }
        accumulator.emitted_message = true;
        vec![message(
            key,
            LlmMessageStatus::Complete,
            accumulator.text.clone(),
            reason,
        )]
    }
}

fn key_from_frame(frame: &SseFrame, value: &Value) -> LlmKey {
    LlmKey::from_frame_indices(
        frame,
        DEFAULT_STREAM_INDEX,
        value
            .get(JSON_FIELD_INDEX)
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_STREAM_INDEX),
    )
}

fn event_type<'a>(frame: &'a SseFrame, value: &'a Value) -> Option<&'a str> {
    value
        .get(JSON_FIELD_TYPE)
        .and_then(Value::as_str)
        .or(frame.event.as_deref())
}

fn message_blocks(value: &Value) -> Vec<(u64, String)> {
    let Some(content) = value.get(JSON_FIELD_CONTENT).and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            if !block
                .get(JSON_FIELD_TYPE)
                .and_then(Value::as_str)
                .is_some_and(|value| value == JSON_TYPE_TEXT)
            {
                return None;
            }
            text_field(block, JSON_FIELD_TEXT)
                .filter(|text| !text.is_empty())
                .map(|text| (index as u64, text.to_string()))
        })
        .collect()
}
