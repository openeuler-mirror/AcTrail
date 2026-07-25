use std::collections::HashMap;

use serde_json::Value;

use crate::capture::{CaptureDirection, SseFrame, WebSocketMessage};

use super::super::model::{
    JSON_FIELD_CONTENT, JSON_FIELD_TEXT, LlmAccumulator, LlmKey, LlmMessageStatus, LlmOutput,
    delta, message, text_field,
};

const JSON_FIELD_DELTA: &str = "delta";
const JSON_FIELD_ITEM: &str = "item";
const JSON_FIELD_OUTPUT: &str = "output";
const JSON_FIELD_PART: &str = "part";
const JSON_FIELD_RESPONSE: &str = "response";
const JSON_FIELD_TYPE: &str = "type";
const EVENT_CONTENT_PART_DONE: &str = "response.content_part.done";
const EVENT_OUTPUT_ITEM_DONE: &str = "response.output_item.done";
const EVENT_OUTPUT_TEXT_DELTA: &str = "response.output_text.delta";
const EVENT_OUTPUT_TEXT_DONE: &str = "response.output_text.done";
const EVENT_RESPONSE_COMPLETED: &str = "response.completed";
const EVENT_RESPONSE_IN_PROGRESS: &str = "response.in_progress";

#[derive(Debug, Default)]
pub(in crate::llm_projection) struct ResponsesParser {
    streams: HashMap<LlmKey, LlmAccumulator>,
}

impl ResponsesParser {
    pub(in crate::llm_projection) fn matches(frame: &SseFrame, value: &Value) -> bool {
        Self::matches_event(event_type(frame, value))
    }

    pub(in crate::llm_projection) fn matches_websocket(value: &Value) -> bool {
        Self::matches_event(value.get(JSON_FIELD_TYPE).and_then(Value::as_str))
    }

    fn matches_event(event: Option<&str>) -> bool {
        matches!(
            event,
            Some(EVENT_OUTPUT_TEXT_DELTA)
                | Some(EVENT_OUTPUT_TEXT_DONE)
                | Some(EVENT_CONTENT_PART_DONE)
                | Some(EVENT_OUTPUT_ITEM_DONE)
                | Some(EVENT_RESPONSE_IN_PROGRESS)
                | Some(EVENT_RESPONSE_COMPLETED)
        )
    }

    pub(in crate::llm_projection) fn push_frame(
        &mut self,
        frame: &SseFrame,
        value: &Value,
    ) -> Vec<LlmOutput> {
        self.push_event(
            LlmKey::from_responses_frame(frame, value),
            event_type(frame, value),
            value,
        )
    }

    pub(in crate::llm_projection) fn push_websocket(
        &mut self,
        message: &WebSocketMessage,
        value: &Value,
    ) -> Vec<LlmOutput> {
        self.push_event(
            LlmKey::from_responses_websocket(message, value),
            value.get(JSON_FIELD_TYPE).and_then(Value::as_str),
            value,
        )
    }

    pub(in crate::llm_projection) fn start_websocket_round(&mut self, pid: u32, stream_key: u64) {
        self.streams.retain(|key, _| {
            key.pid != pid
                || key.stream_key != stream_key
                || key.direction != CaptureDirection::Inbound
        });
    }

    fn push_event(&mut self, key: LlmKey, event: Option<&str>, value: &Value) -> Vec<LlmOutput> {
        match event {
            Some(EVENT_OUTPUT_TEXT_DELTA) => self.push_delta(key, value),
            Some(EVENT_OUTPUT_TEXT_DONE) => self.finish_output_text(key, value),
            Some(EVENT_CONTENT_PART_DONE) => self.validate_content_part(key, value),
            Some(EVENT_OUTPUT_ITEM_DONE) => self.validate_output_item(key, value),
            Some(EVENT_RESPONSE_IN_PROGRESS) => self.validate_response_snapshot(key, value, false),
            Some(EVENT_RESPONSE_COMPLETED) => self.validate_response_snapshot(key, value, true),
            _ => Vec::new(),
        }
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
                Some("target exited before response.output_text.done".to_string()),
            ));
        }
        output
    }

    fn push_delta(&mut self, key: LlmKey, value: &Value) -> Vec<LlmOutput> {
        let Some(delta_text) = text_field(value, JSON_FIELD_DELTA) else {
            return Vec::new();
        };
        self.streams
            .entry(key)
            .or_default()
            .text
            .push_str(delta_text);
        vec![delta(key, delta_text)]
    }

    fn finish_output_text(&mut self, key: LlmKey, value: &Value) -> Vec<LlmOutput> {
        let expected = text_field(value, JSON_FIELD_TEXT);
        self.finish_with_expected(key, expected, None, true)
    }

    fn validate_content_part(&mut self, key: LlmKey, value: &Value) -> Vec<LlmOutput> {
        let expected = value
            .get(JSON_FIELD_PART)
            .and_then(|part| text_field(part, JSON_FIELD_TEXT));
        self.finish_with_expected(key, expected, Some("response.content_part.done"), true)
    }

    fn validate_output_item(&mut self, key: LlmKey, value: &Value) -> Vec<LlmOutput> {
        let expected = output_item_text(value);
        self.finish_with_expected(
            key,
            expected.as_deref(),
            Some("response.output_item.done"),
            true,
        )
    }

    fn validate_response_snapshot(
        &mut self,
        key: LlmKey,
        value: &Value,
        allow_emit_without_delta: bool,
    ) -> Vec<LlmOutput> {
        let expected = response_snapshot_text(value);
        self.finish_with_expected(
            key,
            expected.as_deref(),
            Some("response snapshot"),
            allow_emit_without_delta,
        )
    }

    fn finish_with_expected(
        &mut self,
        key: LlmKey,
        expected: Option<&str>,
        source: Option<&str>,
        allow_emit_without_delta: bool,
    ) -> Vec<LlmOutput> {
        let Some(accumulator) = self.streams.get_mut(&key) else {
            if !allow_emit_without_delta {
                return Vec::new();
            }
            let Some(expected) = expected.filter(|text| !text.is_empty()) else {
                return Vec::new();
            };
            self.streams.insert(
                key,
                LlmAccumulator {
                    text: expected.to_string(),
                    emitted_message: true,
                },
            );
            return vec![message(
                key,
                LlmMessageStatus::Complete,
                expected.to_string(),
                None,
            )];
        };
        if accumulator.emitted_message {
            return Vec::new();
        };
        if accumulator.text.is_empty() {
            let Some(expected) = expected.filter(|text| !text.is_empty()) else {
                return Vec::new();
            };
            accumulator.text.push_str(expected);
            accumulator.emitted_message = true;
            return vec![message(
                key,
                LlmMessageStatus::Complete,
                accumulator.text.clone(),
                None,
            )];
        }
        let status = match expected {
            Some(expected) if expected != accumulator.text => LlmMessageStatus::Inconsistent,
            _ => LlmMessageStatus::Complete,
        };
        let reason = match (status, source, expected) {
            (LlmMessageStatus::Inconsistent, Some(source), Some(_)) => {
                Some(format!("{source} text differs from accumulated deltas"))
            }
            (LlmMessageStatus::Inconsistent, None, Some(_)) => {
                Some("response.output_text.done text differs from accumulated deltas".to_string())
            }
            _ => None,
        };
        accumulator.emitted_message = true;
        vec![message(key, status, accumulator.text.clone(), reason)]
    }
}

fn event_type<'a>(frame: &'a SseFrame, value: &'a Value) -> Option<&'a str> {
    value
        .get(JSON_FIELD_TYPE)
        .and_then(Value::as_str)
        .or(frame.event.as_deref())
}

fn output_item_text(value: &Value) -> Option<String> {
    let Some(content) = value
        .get(JSON_FIELD_ITEM)
        .and_then(|item| item.get(JSON_FIELD_CONTENT))
        .and_then(Value::as_array)
    else {
        return text_field(value, JSON_FIELD_TEXT).map(ToString::to_string);
    };
    let text = content
        .iter()
        .filter_map(|part| text_field(part, JSON_FIELD_TEXT))
        .collect::<Vec<_>>()
        .join("");
    if text.is_empty() { None } else { Some(text) }
}

fn response_snapshot_text(value: &Value) -> Option<String> {
    let response = value.get(JSON_FIELD_RESPONSE).unwrap_or(value);
    let output = response.get(JSON_FIELD_OUTPUT).and_then(Value::as_array)?;
    let text = output
        .iter()
        .filter_map(|item| item.get(JSON_FIELD_CONTENT).and_then(Value::as_array))
        .flat_map(|content| content.iter())
        .filter_map(|part| text_field(part, JSON_FIELD_TEXT))
        .collect::<Vec<_>>()
        .join("");
    if text.is_empty() { None } else { Some(text) }
}
