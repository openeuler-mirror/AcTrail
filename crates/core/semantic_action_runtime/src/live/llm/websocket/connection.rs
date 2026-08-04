//! Responses WebSocket message projection onto the existing HTTP LLM seam.

use model_core::payload::{PayloadDirection, PayloadSegment, PayloadStreamKey};
use serde_json::Value;

use super::framing::{DirectionAssembler, MAX_DECODED_BYTES};
use super::handshake::NegotiatedExtensions;

pub(super) struct WebSocketConnection {
    stream_key: PayloadStreamKey,
    synthetic_stream_key: PayloadStreamKey,
    path: String,
    outbound: DirectionAssembler,
    inbound: DirectionAssembler,
    response_text: String,
}

pub(super) struct ConnectionObservation {
    pub(super) projected: Vec<PayloadSegment>,
    pub(super) closed: bool,
}

impl WebSocketConnection {
    pub(super) fn new(
        stream_key: PayloadStreamKey,
        path: String,
        extensions: NegotiatedExtensions,
    ) -> Self {
        Self {
            synthetic_stream_key: PayloadStreamKey::new(format!("websocket:{stream_key}")),
            stream_key,
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
        }
    }

    pub(super) fn observe(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Option<ConnectionObservation>, ()> {
        if self.stream_key != segment.stream_key {
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
                PayloadDirection::Inbound => self.project_inbound(segment, &value, &mut projected),
            }
        }
        Ok(Some(ConnectionObservation {
            projected,
            closed: assembled.closed,
        }))
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
        self.response_text.clear();
        let body = text.as_bytes();
        let mut bytes = format!(
            "POST {} HTTP/1.1\r\nHost: chatgpt.com\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.path,
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(body);
        projected.push(self.synthetic_segment(segment, bytes));
    }

    fn project_inbound(
        &mut self,
        segment: &PayloadSegment,
        value: &Value,
        projected: &mut Vec<PayloadSegment>,
    ) {
        let Some(message_type) = value.get("type").and_then(Value::as_str) else {
            return;
        };
        if !message_type.starts_with("response.") {
            return;
        }
        if message_type == "response.output_text.delta" {
            if let Some(delta) = value.get("delta").and_then(Value::as_str)
                && self.response_text.len().saturating_add(delta.len()) <= MAX_DECODED_BYTES
            {
                self.response_text.push_str(delta);
            }
            return;
        }
        if message_type == "response.output_text.done"
            && self.response_text.is_empty()
            && let Some(done_text) = value.get("text").and_then(Value::as_str)
            && done_text.len() <= MAX_DECODED_BYTES
        {
            self.response_text.push_str(done_text);
        }
        if !matches!(
            message_type,
            "response.completed" | "response.failed" | "response.incomplete"
        ) {
            return;
        }
        let Some(mut response) = value.get("response").cloned() else {
            self.response_text.clear();
            return;
        };
        Self::ensure_response_output(&mut response, &self.response_text);
        let Ok(body) = serde_json::to_vec(&response) else {
            self.response_text.clear();
            return;
        };
        self.response_text.clear();
        let mut bytes = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(&body);
        projected.push(self.synthetic_segment(segment, bytes));
    }

    fn ensure_response_output(response: &mut Value, text: &str) {
        let output_has_content = response
            .get("output")
            .and_then(Value::as_array)
            .is_some_and(|output| !output.is_empty());
        if output_has_content {
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

    fn synthetic_segment(&self, source: &PayloadSegment, bytes: Vec<u8>) -> PayloadSegment {
        let size = bytes.len() as u64;
        let mut segment = source.clone();
        segment.stream_key = self.synthetic_stream_key.clone();
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
