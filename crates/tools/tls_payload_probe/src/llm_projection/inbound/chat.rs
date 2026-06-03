use std::collections::HashMap;

use serde_json::Value;

use crate::capture::SseFrame;

use super::super::model::{
    DEFAULT_STREAM_INDEX, JSON_FIELD_CONTENT, LlmAccumulator, LlmKey, LlmMessageStatus, LlmOutput,
    delta, message, text_field,
};

const CHAT_DONE_DATA: &str = "[DONE]";
const JSON_FIELD_CHOICES: &str = "choices";
const JSON_FIELD_DELTA: &str = "delta";
const JSON_FIELD_FINISH_REASON: &str = "finish_reason";
const JSON_FIELD_INDEX: &str = "index";
const JSON_FIELD_MESSAGE: &str = "message";

#[derive(Debug, Default)]
pub(in crate::llm_projection) struct ChatCompletionsParser {
    streams: HashMap<LlmKey, LlmAccumulator>,
}

impl ChatCompletionsParser {
    pub(in crate::llm_projection) fn matches(data: &str, value: Option<&Value>) -> bool {
        if data.trim() == CHAT_DONE_DATA {
            return false;
        }
        value.is_some_and(has_chat_choices)
    }

    pub(in crate::llm_projection) fn push_frame(
        &mut self,
        frame: &SseFrame,
        data: &str,
        value: Option<&Value>,
    ) -> Vec<LlmOutput> {
        if data.trim() == CHAT_DONE_DATA {
            return self.finish_stream(frame);
        }
        let Some(value) = value else {
            return Vec::new();
        };
        let Some(choices) = value.get(JSON_FIELD_CHOICES).and_then(Value::as_array) else {
            return Vec::new();
        };
        let mut output = Vec::new();
        for choice in choices {
            let key = LlmKey::from_frame_indices(
                frame,
                choice
                    .get(JSON_FIELD_INDEX)
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_STREAM_INDEX),
                DEFAULT_STREAM_INDEX,
            );
            if let Some(content) = choice
                .get(JSON_FIELD_DELTA)
                .and_then(|delta| text_field(delta, JSON_FIELD_CONTENT))
                .filter(|content| !content.is_empty())
            {
                self.streams.entry(key).or_default().text.push_str(content);
                output.push(delta(key, content));
            }
            if let Some(content) = choice
                .get(JSON_FIELD_MESSAGE)
                .and_then(|message| text_field(message, JSON_FIELD_CONTENT))
                .filter(|content| !content.is_empty())
            {
                output.extend(self.finish_with_content(key, content));
                continue;
            }
            if choice
                .get(JSON_FIELD_FINISH_REASON)
                .is_some_and(|reason| !reason.is_null())
            {
                output.extend(self.finish_choice(key));
            }
        }
        output
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
                Some("target exited before chat completion stream finished".to_string()),
            ));
        }
        output
    }

    fn finish_stream(&mut self, frame: &SseFrame) -> Vec<LlmOutput> {
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
            .flat_map(|key| self.finish_choice(key))
            .collect()
    }

    fn finish_choice(&mut self, key: LlmKey) -> Vec<LlmOutput> {
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
            None,
        )]
    }

    fn finish_with_content(&mut self, key: LlmKey, content: &str) -> Vec<LlmOutput> {
        let accumulator = self.streams.entry(key).or_default();
        if accumulator.emitted_message {
            return Vec::new();
        }
        if accumulator.text.is_empty() {
            accumulator.text.push_str(content);
        }
        let status = if accumulator.text == content {
            LlmMessageStatus::Complete
        } else {
            LlmMessageStatus::Inconsistent
        };
        let reason = (status == LlmMessageStatus::Inconsistent)
            .then(|| "chat completion message content differs from accumulated deltas".to_string());
        accumulator.emitted_message = true;
        vec![message(key, status, accumulator.text.clone(), reason)]
    }
}

fn has_chat_choices(value: &Value) -> bool {
    value
        .get(JSON_FIELD_CHOICES)
        .and_then(Value::as_array)
        .is_some_and(|choices| choices.iter().any(is_chat_choice))
}

fn is_chat_choice(choice: &Value) -> bool {
    choice.get(JSON_FIELD_DELTA).is_some()
        || choice.get(JSON_FIELD_MESSAGE).is_some()
        || choice.get(JSON_FIELD_FINISH_REASON).is_some()
}
