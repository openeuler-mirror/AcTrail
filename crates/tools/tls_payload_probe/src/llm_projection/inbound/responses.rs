use std::collections::HashMap;

use serde_json::Value;

use crate::capture::SseFrame;

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
        matches!(
            event_type(frame, value),
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
        match event_type(frame, value) {
            Some(EVENT_OUTPUT_TEXT_DELTA) => self.push_delta(frame, value),
            Some(EVENT_OUTPUT_TEXT_DONE) => self.finish_output_text(frame, value),
            Some(EVENT_CONTENT_PART_DONE) => self.validate_content_part(frame, value),
            Some(EVENT_OUTPUT_ITEM_DONE) => self.validate_output_item(frame, value),
            Some(EVENT_RESPONSE_IN_PROGRESS) => {
                self.validate_response_snapshot(frame, value, false)
            }
            Some(EVENT_RESPONSE_COMPLETED) => self.validate_response_snapshot(frame, value, true),
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

    fn push_delta(&mut self, frame: &SseFrame, value: &Value) -> Vec<LlmOutput> {
        let Some(delta_text) = text_field(value, JSON_FIELD_DELTA) else {
            return Vec::new();
        };
        let key = LlmKey::from_responses_frame(frame, value);
        self.streams
            .entry(key)
            .or_default()
            .text
            .push_str(delta_text);
        vec![delta(key, delta_text)]
    }

    fn finish_output_text(&mut self, frame: &SseFrame, value: &Value) -> Vec<LlmOutput> {
        let key = LlmKey::from_responses_frame(frame, value);
        let expected = text_field(value, JSON_FIELD_TEXT);
        self.finish_with_expected(key, expected, None, true)
    }

    fn validate_content_part(&mut self, frame: &SseFrame, value: &Value) -> Vec<LlmOutput> {
        let key = LlmKey::from_responses_frame(frame, value);
        let expected = value
            .get(JSON_FIELD_PART)
            .and_then(|part| text_field(part, JSON_FIELD_TEXT));
        self.finish_with_expected(key, expected, Some("response.content_part.done"), true)
    }

    fn validate_output_item(&mut self, frame: &SseFrame, value: &Value) -> Vec<LlmOutput> {
        let key = LlmKey::from_responses_frame(frame, value);
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
        frame: &SseFrame,
        value: &Value,
        allow_emit_without_delta: bool,
    ) -> Vec<LlmOutput> {
        let key = LlmKey::from_responses_frame(frame, value);
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
