use serde_json::Value;

use crate::capture::{CaptureDirection, SseFrame};

pub(super) const DEFAULT_STREAM_INDEX: u64 = 0;
pub(super) const JSON_FIELD_CONTENT: &str = "content";
pub(super) const JSON_FIELD_CONTENT_INDEX: &str = "content_index";
pub(super) const JSON_FIELD_OUTPUT_INDEX: &str = "output_index";
pub(super) const JSON_FIELD_TEXT: &str = "text";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LlmOutput {
    Request(LlmRequest),
    Delta(LlmDelta),
    Message(LlmMessage),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LlmRequest {
    pub(crate) pid: u32,
    pub(crate) stream_key: u64,
    pub(crate) direction: CaptureDirection,
    pub(crate) schema: LlmRequestSchema,
    pub(crate) model: Option<String>,
    pub(crate) stream: Option<bool>,
    pub(crate) items: Vec<LlmRequestItem>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LlmRequestSchema {
    AnthropicMessages,
    OpenAiChatCompletions,
    OpenAiResponses,
}

impl LlmRequestSchema {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AnthropicMessages => "anthropic_messages",
            Self::OpenAiChatCompletions => "openai_chat_completions",
            Self::OpenAiResponses => "openai_responses",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LlmRequestItem {
    pub(crate) label: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LlmDelta {
    pub(crate) pid: u32,
    pub(crate) stream_key: u64,
    pub(crate) direction: CaptureDirection,
    pub(crate) output_index: u64,
    pub(crate) content_index: u64,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LlmMessage {
    pub(crate) pid: u32,
    pub(crate) stream_key: u64,
    pub(crate) direction: CaptureDirection,
    pub(crate) output_index: u64,
    pub(crate) content_index: u64,
    pub(crate) status: LlmMessageStatus,
    pub(crate) reason: Option<String>,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LlmMessageStatus {
    Complete,
    Partial,
    Inconsistent,
}

impl LlmMessageStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Inconsistent => "inconsistent",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct LlmKey {
    pub(super) pid: u32,
    pub(super) stream_key: u64,
    pub(super) direction: CaptureDirection,
    pub(super) output_index: u64,
    pub(super) content_index: u64,
}

impl LlmKey {
    pub(super) fn from_indices(
        pid: u32,
        stream_key: u64,
        direction: CaptureDirection,
        output_index: u64,
        content_index: u64,
    ) -> Self {
        Self {
            pid,
            stream_key,
            direction,
            output_index,
            content_index,
        }
    }

    pub(super) fn from_frame_indices(
        frame: &SseFrame,
        output_index: u64,
        content_index: u64,
    ) -> Self {
        Self::from_indices(
            frame.pid,
            frame.stream_key,
            frame.direction,
            output_index,
            content_index,
        )
    }

    pub(super) fn from_responses_frame(frame: &SseFrame, value: &Value) -> Self {
        Self::from_frame_indices(
            frame,
            value
                .get(JSON_FIELD_OUTPUT_INDEX)
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_STREAM_INDEX),
            value
                .get(JSON_FIELD_CONTENT_INDEX)
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_STREAM_INDEX),
        )
    }
}

#[derive(Debug, Default)]
pub(super) struct LlmAccumulator {
    pub(super) text: String,
    pub(super) emitted_message: bool,
}

pub(super) fn delta(key: LlmKey, text: &str) -> LlmOutput {
    LlmOutput::Delta(LlmDelta {
        pid: key.pid,
        stream_key: key.stream_key,
        direction: key.direction,
        output_index: key.output_index,
        content_index: key.content_index,
        text: text.to_string(),
    })
}

pub(super) fn request(
    pid: u32,
    stream_key: u64,
    direction: CaptureDirection,
    schema: LlmRequestSchema,
    model: Option<String>,
    stream: Option<bool>,
    items: Vec<LlmRequestItem>,
) -> LlmOutput {
    LlmOutput::Request(LlmRequest {
        pid,
        stream_key,
        direction,
        schema,
        model,
        stream,
        items,
    })
}

pub(super) fn message(
    key: LlmKey,
    status: LlmMessageStatus,
    text: String,
    reason: Option<String>,
) -> LlmOutput {
    LlmOutput::Message(LlmMessage {
        pid: key.pid,
        stream_key: key.stream_key,
        direction: key.direction,
        output_index: key.output_index,
        content_index: key.content_index,
        status,
        reason,
        text,
    })
}

pub(super) fn text_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}
