//! Responses WebSocket message projection onto the existing HTTP LLM seam.

use std::time::SystemTime;

use model_core::payload::{PayloadDirection, PayloadSegment, PayloadStreamKey};
use serde_json::Value;

use super::framing::{DirectionAssembler, MAX_DECODED_BYTES};
use super::handshake::NegotiatedExtensions;

pub(super) struct WebSocketConnection {
    stream_key: Option<PayloadStreamKey>,
    synthetic_stream_key_prefix: String,
    next_exchange_id: u64,
    active_exchange_stream_key: Option<PayloadStreamKey>,
    path: String,
    outbound: DirectionAssembler,
    inbound: DirectionAssembler,
    response_text: String,
    response_output: Vec<Value>,
    response_output_bytes: usize,
    response_started_at: Option<SystemTime>,
}

pub(super) struct ConnectionObservation {
    pub(super) projected: Vec<PayloadSegment>,
    pub(super) closed: bool,
}

impl WebSocketConnection {
    pub(super) fn new(
        outbound_stream_key: PayloadStreamKey,
        inbound_stream_key: PayloadStreamKey,
        path: String,
        extensions: NegotiatedExtensions,
    ) -> Self {
        Self {
            synthetic_stream_key_prefix: format!(
                "websocket:{outbound_stream_key}:{inbound_stream_key}:exchange"
            ),
            next_exchange_id: 0,
            active_exchange_stream_key: None,
            stream_key: None,
            path,
            outbound: DirectionAssembler::new(
                true,
                extensions.permessage_deflate,
                extensions.client_no_context_takeover,
            ),
            inbound: DirectionAssembler::new(
                false,
                extensions.permessage_deflate,
                extensions.server_no_context_takeover,
            ),
            response_text: String::new(),
            response_output: Vec::new(),
            response_output_bytes: 0,
            response_started_at: None,
        }
    }

    pub(super) fn observe(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Option<ConnectionObservation>, ()> {
        if !self.accepts_segment(segment) {
            return Ok(None);
        }
        let assembled = match segment.direction {
            PayloadDirection::Outbound => self.outbound.push(&segment.bytes)?,
            PayloadDirection::Inbound => self.inbound.push(&segment.bytes)?,
        };
        let mut projected = Vec::new();
        for payload in assembled.messages {
            let Ok(text) = String::from_utf8(payload) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            match segment.direction {
                PayloadDirection::Outbound => {
                    self.project_outbound(segment, &text, &value, &mut projected)
                }
                PayloadDirection::Inbound => {
                    self.project_inbound(segment, &value, &mut projected)?
                }
            }
        }
        Ok(Some(ConnectionObservation {
            projected,
            closed: assembled.closed,
        }))
    }

    pub(super) fn is_bound_to(&self, stream_key: &PayloadStreamKey) -> bool {
        self.stream_key.as_ref() == Some(stream_key)
    }

    pub(super) fn synthetic_stream_key_prefix(&self) -> &str {
        &self.synthetic_stream_key_prefix
    }

    fn accepts_segment(&mut self, segment: &PayloadSegment) -> bool {
        if let Some(stream_key) = self.stream_key.as_ref() {
            return stream_key == &segment.stream_key;
        }
        let expected_masked = segment.direction == PayloadDirection::Outbound;
        if !super::framing::FrameDecoder::looks_like_frame(&segment.bytes, expected_masked) {
            return false;
        }
        self.stream_key = Some(segment.stream_key.clone());
        true
    }

    fn project_outbound(
        &mut self,
        segment: &PayloadSegment,
        text: &str,
        value: &Value,
        projected: &mut Vec<PayloadSegment>,
    ) {
        if value.get("type").and_then(Value::as_str) != Some("response.create") {
            return;
        }
        self.clear_response();
        let stream_key = PayloadStreamKey::new(format!(
            "{}:{}",
            self.synthetic_stream_key_prefix, self.next_exchange_id
        ));
        self.next_exchange_id = self.next_exchange_id.saturating_add(1);
        self.active_exchange_stream_key = Some(stream_key.clone());
        let body = text.as_bytes();
        let mut bytes = format!(
            "POST {} HTTP/1.1\r\nHost: chatgpt.com\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.path,
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(body);
        projected.push(self.synthetic_segment(segment, stream_key, bytes));
    }

    fn project_inbound(
        &mut self,
        segment: &PayloadSegment,
        value: &Value,
        projected: &mut Vec<PayloadSegment>,
    ) -> Result<(), ()> {
        let Some(message_type) = value.get("type").and_then(Value::as_str) else {
            return Ok(());
        };
        if !message_type.starts_with("response.") {
            return Ok(());
        }
        self.response_started_at.get_or_insert(segment.observed_at);
        if message_type == "response.output_text.delta" {
            if let Some(delta) = value.get("delta").and_then(Value::as_str)
                && self.response_text.len().saturating_add(delta.len()) <= MAX_DECODED_BYTES
            {
                self.response_text.push_str(delta);
            }
            return Ok(());
        }
        if message_type == "response.output_text.done"
            && self.response_text.is_empty()
            && let Some(done_text) = value.get("text").and_then(Value::as_str)
            && done_text.len() <= MAX_DECODED_BYTES
        {
            self.response_text.push_str(done_text);
        }
        if message_type == "response.output_item.done" {
            self.capture_response_output_item(value)?;
            return Ok(());
        }
        if !matches!(
            message_type,
            "response.completed" | "response.failed" | "response.incomplete"
        ) {
            return Ok(());
        }
        let Some(mut response) = value.get("response").cloned() else {
            self.clear_response();
            self.active_exchange_stream_key = None;
            return Ok(());
        };
        Self::ensure_response_output(&mut response, &self.response_output, &self.response_text);
        let Ok(body) = serde_json::to_vec(&response) else {
            self.clear_response();
            return Ok(());
        };
        let mut bytes = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(&body);
        let Some(stream_key) = self.active_exchange_stream_key.take() else {
            self.clear_response();
            return Ok(());
        };
        let mut synthetic = self.synthetic_segment(segment, stream_key, bytes);
        if let Some(response_started_at) = self.response_started_at {
            synthetic.observed_at = response_started_at;
        }
        self.clear_response();
        projected.push(synthetic);
        Ok(())
    }

    fn capture_response_output_item(&mut self, value: &Value) -> Result<(), ()> {
        let item = value.get("item").ok_or(())?;
        let item_bytes = serde_json::to_vec(item).map_err(|_| ())?.len();
        let response_output_bytes = self
            .response_output_bytes
            .checked_add(item_bytes)
            .filter(|bytes| *bytes <= MAX_DECODED_BYTES)
            .ok_or(())?;
        self.response_output.push(item.clone());
        self.response_output_bytes = response_output_bytes;
        Ok(())
    }

    fn clear_response(&mut self) {
        self.response_text.clear();
        self.response_output.clear();
        self.response_output_bytes = 0;
        self.response_started_at = None;
    }

    fn ensure_response_output(response: &mut Value, output_items: &[Value], text: &str) {
        if let Some(output) = response.get_mut("output").and_then(Value::as_array_mut)
            && !output.is_empty()
        {
            Self::merge_captured_tool_calls(output, output_items);
            return;
        }
        if !output_items.is_empty() {
            response["output"] = Value::Array(output_items.to_vec());
            return;
        }
        response["output"] = serde_json::json!([{
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{
                "type": "output_text",
                "text": text
            }]
        }]);
    }

    fn merge_captured_tool_calls(output: &mut Vec<Value>, captured: &[Value]) {
        for item in captured.iter().filter(|item| {
            matches!(
                item.get("type").and_then(Value::as_str),
                Some("function_call" | "custom_tool_call")
            )
        }) {
            let duplicate = output
                .iter()
                .filter(|existing| {
                    matches!(
                        existing.get("type").and_then(Value::as_str),
                        Some("function_call" | "custom_tool_call")
                    )
                })
                .any(|existing| {
                    ["call_id", "id"].into_iter().any(|key| {
                        item.get(key)
                            .and_then(Value::as_str)
                            .is_some_and(|id| existing.get(key).and_then(Value::as_str) == Some(id))
                    }) || existing == item
                });
            if !duplicate {
                output.push(item.clone());
            }
        }
    }

    fn synthetic_segment(
        &self,
        source: &PayloadSegment,
        stream_key: PayloadStreamKey,
        bytes: Vec<u8>,
    ) -> PayloadSegment {
        let size = bytes.len() as u64;
        let mut segment = source.clone();
        segment.stream_key = stream_key;
        segment.original_size = size;
        segment.captured_size = size;
        segment.operation_original_size = size;
        segment.operation_captured_size = size;
        segment.operation_offset = 0;
        segment.library = "websocket".to_string();
        segment.symbol = "message".to_string();
        segment.protocol_hint = Some("websocket.responses".to_string());
        segment.bytes = bytes;
        segment
    }
}
