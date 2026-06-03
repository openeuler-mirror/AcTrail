use std::collections::HashMap;

use serde_json::Value;

use crate::capture::{
    AssembledHttp, CaptureConfig, CaptureDirection, HttpBody, HttpBodyFragment, HttpDecodeConfig,
    SseFrame, decoded_text_from_headers,
};
use crate::{ToolError, ToolResult};

use super::inbound::{AnthropicMessagesParser, ChatCompletionsParser, ResponsesParser};
use super::model::{DEFAULT_STREAM_INDEX, LlmKey, LlmOutput};
use super::outbound::{
    AnthropicMessagesRequestParser, ChatCompletionsRequestParser, ResponsesRequestParser,
};

#[derive(Debug)]
pub(crate) struct LlmProjector {
    routes: HashMap<LlmStreamKey, ParserKind>,
    http_bodies: HashMap<HttpStreamKey, Vec<u8>>,
    responses: ResponsesParser,
    chat_completions: ChatCompletionsParser,
    anthropic_messages: AnthropicMessagesParser,
    decode: HttpDecodeConfig,
    max_http_body_bytes: usize,
}

impl LlmProjector {
    pub(crate) fn new(config: &CaptureConfig) -> Self {
        Self {
            routes: HashMap::new(),
            http_bodies: HashMap::new(),
            responses: ResponsesParser::default(),
            chat_completions: ChatCompletionsParser::default(),
            anthropic_messages: AnthropicMessagesParser::default(),
            decode: HttpDecodeConfig::from(config),
            max_http_body_bytes: config.assemble_buffer_bytes,
        }
    }

    pub(crate) fn push_frame(&mut self, frame: &SseFrame, data: &str) -> Vec<LlmOutput> {
        if frame.direction != CaptureDirection::Inbound {
            return Vec::new();
        }
        let key = LlmStreamKey::from_frame(frame);
        let parsed = serde_json::from_str::<Value>(data).ok();
        if let Some(kind) = self.routes.get(&key).copied() {
            return self.dispatch(kind, frame, data, parsed.as_ref());
        }
        let Some(kind) = detect_parser(frame, data, parsed.as_ref()) else {
            return Vec::new();
        };
        self.routes.insert(key, kind);
        self.dispatch(kind, frame, data, parsed.as_ref())
    }

    pub(crate) fn push_http_fragment(
        &mut self,
        fragment: &HttpBodyFragment,
    ) -> ToolResult<Vec<LlmOutput>> {
        if fragment.is_event_stream() {
            return Ok(Vec::new());
        }
        let key = HttpStreamKey::from_fragment(fragment);
        let body = self.http_bodies.entry(key).or_default();
        let next_len = body
            .len()
            .checked_add(fragment.body_bytes().len())
            .ok_or_else(|| ToolError::new("LLM projection HTTP body buffer length overflow"))?;
        if next_len > self.max_http_body_bytes {
            return Err(ToolError::new(format!(
                "LLM projection HTTP body buffer exceeded {} bytes for pid={} stream=0x{:x} direction={}",
                self.max_http_body_bytes,
                key.pid,
                key.stream_key,
                key.direction.as_str()
            )));
        }
        body.extend_from_slice(fragment.body_bytes());
        Ok(Vec::new())
    }

    pub(crate) fn push_http_message(
        &mut self,
        message: &AssembledHttp,
    ) -> ToolResult<Vec<LlmOutput>> {
        let key = HttpStreamKey::from_message(message);
        if message.is_event_stream() {
            self.http_bodies.remove(&key);
            return Ok(Vec::new());
        }
        let Some(text) = self.http_message_text(message, key)? else {
            return Ok(Vec::new());
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return Ok(Vec::new());
        };
        if message.direction == CaptureDirection::Outbound {
            return Ok(project_outbound_request(message, &value));
        }
        if !AnthropicMessagesParser::matches_http(&value) {
            return Ok(Vec::new());
        }
        Ok(self.anthropic_messages.push_http_message(
            LlmKey::from_indices(
                message.pid,
                message.stream_key,
                message.direction,
                DEFAULT_STREAM_INDEX,
                DEFAULT_STREAM_INDEX,
            ),
            &value,
        ))
    }

    pub(crate) fn finish(&mut self) -> Vec<LlmOutput> {
        let mut output = self.responses.finish();
        output.extend(self.chat_completions.finish());
        output.extend(self.anthropic_messages.finish());
        self.routes.clear();
        self.http_bodies.clear();
        output
    }

    fn dispatch(
        &mut self,
        kind: ParserKind,
        frame: &SseFrame,
        data: &str,
        value: Option<&Value>,
    ) -> Vec<LlmOutput> {
        match kind {
            ParserKind::Responses => {
                let Some(value) = value else {
                    return Vec::new();
                };
                self.responses.push_frame(frame, value)
            }
            ParserKind::ChatCompletions => self.chat_completions.push_frame(frame, data, value),
            ParserKind::AnthropicMessages => {
                let Some(value) = value else {
                    return Vec::new();
                };
                self.anthropic_messages.push_frame(frame, value)
            }
        }
    }

    fn http_message_text(
        &mut self,
        message: &AssembledHttp,
        key: HttpStreamKey,
    ) -> ToolResult<Option<String>> {
        if let Some(text) = message.body_text() {
            self.http_bodies.remove(&key);
            return Ok(Some(text.to_string()));
        }
        match &message.body {
            HttpBody::Streamed { .. } => {
                let Some(body) = self.http_bodies.remove(&key) else {
                    return Ok(None);
                };
                decoded_text_from_headers(&body, &message.headers, &self.decode)
            }
            HttpBody::Empty
            | HttpBody::Binary { .. }
            | HttpBody::DecodedBinary { .. }
            | HttpBody::DecodeSkipped { .. }
            | HttpBody::DecodeFailed { .. }
            | HttpBody::Partial { .. } => {
                self.http_bodies.remove(&key);
                Ok(None)
            }
            HttpBody::Text { .. }
            | HttpBody::DecodedText { .. }
            | HttpBody::PartialText { .. }
            | HttpBody::PartialDecodedText { .. } => unreachable!("text bodies returned earlier"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct LlmStreamKey {
    pid: u32,
    stream_key: u64,
    direction: CaptureDirection,
}

impl LlmStreamKey {
    fn from_frame(frame: &SseFrame) -> Self {
        Self {
            pid: frame.pid,
            stream_key: frame.stream_key,
            direction: frame.direction,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct HttpStreamKey {
    pid: u32,
    stream_key: u64,
    direction: CaptureDirection,
}

impl HttpStreamKey {
    fn from_fragment(fragment: &HttpBodyFragment) -> Self {
        Self {
            pid: fragment.pid,
            stream_key: fragment.stream_key,
            direction: fragment.direction,
        }
    }

    fn from_message(message: &AssembledHttp) -> Self {
        Self {
            pid: message.pid,
            stream_key: message.stream_key,
            direction: message.direction,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParserKind {
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

fn detect_parser(frame: &SseFrame, data: &str, value: Option<&Value>) -> Option<ParserKind> {
    if let Some(value) = value
        && ResponsesParser::matches(frame, value)
    {
        return Some(ParserKind::Responses);
    }
    if ChatCompletionsParser::matches(data, value) {
        return Some(ParserKind::ChatCompletions);
    }
    if let Some(value) = value
        && AnthropicMessagesParser::matches_frame(frame, value)
    {
        return Some(ParserKind::AnthropicMessages);
    }
    None
}

fn project_outbound_request(message: &AssembledHttp, value: &Value) -> Vec<LlmOutput> {
    ResponsesRequestParser::parse(message, value)
        .or_else(|| ChatCompletionsRequestParser::parse(message, value))
        .or_else(|| AnthropicMessagesRequestParser::parse(message, value))
        .into_iter()
        .collect()
}
