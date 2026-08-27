//! Responses WebSocket message projection onto the existing HTTP LLM seam.

use std::time::SystemTime;

use model_core::payload::{
    PayloadDirection, PayloadSegment, PayloadStreamKey, PayloadTruncationState,
};
use serde_json::Value;

use super::framing::DirectionAssembler;
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
    max_response_bytes: usize,
    discarding_response_until_terminal: bool,
    last_source: Option<PayloadSegment>,
}

#[derive(Default)]
pub(super) struct ConnectionObservation {
    pub(super) projected: Vec<PayloadSegment>,
    pub(super) completed_exchange_streams: Vec<PayloadStreamKey>,
    pub(super) partial_exchange_streams: Vec<PayloadStreamKey>,
    pub(super) oversized_response_discarded_bytes: u64,
    pub(super) superseded_responses: u64,
    pub(super) decode_failed_entries: u64,
    pub(super) decode_discarded_bytes: u64,
    pub(super) lifecycle_gap_entries: u64,
    pub(super) closed: bool,
}

impl WebSocketConnection {
    pub(super) fn new(
        outbound_stream_key: PayloadStreamKey,
        inbound_stream_key: PayloadStreamKey,
        path: String,
        extensions: NegotiatedExtensions,
        max_response_bytes: usize,
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
            max_response_bytes,
            discarding_response_until_terminal: false,
            last_source: None,
        }
    }

    pub(super) fn observe(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Option<ConnectionObservation>, ()> {
        if !self.accepts_segment(segment) {
            return Ok(None);
        }
        let mut source = segment.clone();
        source.bytes.clear();
        self.last_source = Some(source);
        let assembled = match segment.direction {
            PayloadDirection::Outbound => self.outbound.push(&segment.bytes)?,
            PayloadDirection::Inbound => self.inbound.push(&segment.bytes)?,
        };
        let mut observation = ConnectionObservation {
            closed: assembled.closed,
            ..ConnectionObservation::default()
        };
        for payload in assembled.messages {
            let payload_bytes = payload.len();
            let Ok(text) = String::from_utf8(payload) else {
                observation.decode_failed_entries =
                    observation.decode_failed_entries.saturating_add(1);
                observation.decode_discarded_bytes = observation
                    .decode_discarded_bytes
                    .saturating_add(payload_bytes as u64);
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                observation.decode_failed_entries =
                    observation.decode_failed_entries.saturating_add(1);
                observation.decode_discarded_bytes = observation
                    .decode_discarded_bytes
                    .saturating_add(payload_bytes as u64);
                continue;
            };
            match segment.direction {
                PayloadDirection::Outbound => {
                    self.project_outbound(segment, &text, &value, &mut observation)
                }
                PayloadDirection::Inbound => {
                    self.project_inbound(segment, &value, text.len(), &mut observation)?
                }
            }
        }
        if assembled.closed && self.active_exchange_stream_key.is_some() {
            self.materialize_partial_response(segment, &mut observation);
            observation.lifecycle_gap_entries = observation.lifecycle_gap_entries.saturating_add(1);
        }
        Ok(Some(observation))
    }

    pub(super) fn materialize_decode_failure(
        &mut self,
        segment: &PayloadSegment,
    ) -> ConnectionObservation {
        let mut observation = ConnectionObservation {
            decode_failed_entries: 1,
            decode_discarded_bytes: segment.bytes.len() as u64,
            ..ConnectionObservation::default()
        };
        self.materialize_partial_response(segment, &mut observation);
        observation
    }

    pub(super) fn materialize_lifecycle_gap(
        &mut self,
        segment: &PayloadSegment,
    ) -> ConnectionObservation {
        let mut observation = ConnectionObservation {
            lifecycle_gap_entries: u64::from(self.active_exchange_stream_key.is_some()),
            ..ConnectionObservation::default()
        };
        self.materialize_partial_response(segment, &mut observation);
        observation
    }

    pub(super) fn materialize_trace_close(
        &mut self,
        finished_at: SystemTime,
    ) -> Option<ConnectionObservation> {
        let mut source = self.last_source.take()?;
        source.observed_at = finished_at;
        Some(self.materialize_lifecycle_gap(&source))
    }

    pub(super) fn retained_response_bytes(&self) -> usize {
        self.response_text
            .len()
            .saturating_add(self.response_output_bytes)
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
        observation: &mut ConnectionObservation,
    ) {
        if value.get("type").and_then(Value::as_str) != Some("response.create") {
            return;
        }
        if self.active_exchange_stream_key.is_some() {
            self.materialize_partial_response(segment, observation);
            observation.superseded_responses = observation.superseded_responses.saturating_add(1);
        }
        self.discarding_response_until_terminal = false;
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
        observation.projected.push(self.synthetic_segment(
            segment,
            stream_key,
            PayloadDirection::Outbound,
            bytes,
        ));
    }

    fn project_inbound(
        &mut self,
        segment: &PayloadSegment,
        value: &Value,
        message_bytes: usize,
        observation: &mut ConnectionObservation,
    ) -> Result<(), ()> {
        let Some(message_type) = value.get("type").and_then(Value::as_str) else {
            return Ok(());
        };
        if !message_type.starts_with("response.") {
            return Ok(());
        }
        if self.active_exchange_stream_key.is_none() {
            if self.discarding_response_until_terminal {
                observation.oversized_response_discarded_bytes = observation
                    .oversized_response_discarded_bytes
                    .saturating_add(message_bytes as u64);
                if is_terminal_response_message(message_type) {
                    self.discarding_response_until_terminal = false;
                }
            }
            return Ok(());
        }
        self.response_started_at.get_or_insert(segment.observed_at);
        if message_type == "response.output_text.delta" {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                self.append_response_text(segment, delta, observation);
            }
            return Ok(());
        }
        if message_type == "response.output_text.done"
            && self.response_text.is_empty()
            && let Some(done_text) = value.get("text").and_then(Value::as_str)
        {
            self.append_response_text(segment, done_text, observation);
            return Ok(());
        }
        if message_type == "response.output_item.done" {
            self.capture_response_output_item(segment, value, observation)?;
            return Ok(());
        }
        if !is_terminal_response_message(message_type) {
            return Ok(());
        }
        let Some(mut response) = value.get("response").cloned() else {
            observation.decode_failed_entries = observation.decode_failed_entries.saturating_add(1);
            observation.decode_discarded_bytes = observation
                .decode_discarded_bytes
                .saturating_add(message_bytes as u64);
            self.materialize_partial_response(segment, observation);
            self.discarding_response_until_terminal = false;
            return Ok(());
        };
        Self::ensure_response_output(&mut response, &self.response_output, &self.response_text);
        let Ok(body) = serde_json::to_vec(&response) else {
            observation.decode_failed_entries = observation.decode_failed_entries.saturating_add(1);
            observation.decode_discarded_bytes = observation
                .decode_discarded_bytes
                .saturating_add(message_bytes as u64);
            self.materialize_partial_response(segment, observation);
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
        let mut synthetic = self.synthetic_segment(
            segment,
            stream_key.clone(),
            PayloadDirection::Inbound,
            bytes,
        );
        if let Some(response_started_at) = self.response_started_at {
            synthetic.observed_at = response_started_at;
        }
        self.clear_response();
        observation.projected.push(synthetic);
        observation.completed_exchange_streams.push(stream_key);
        self.discarding_response_until_terminal = false;
        Ok(())
    }

    fn append_response_text(
        &mut self,
        segment: &PayloadSegment,
        text: &str,
        observation: &mut ConnectionObservation,
    ) {
        let remaining = self
            .max_response_bytes
            .saturating_sub(self.response_text.len())
            .saturating_sub(self.response_output_bytes);
        if text.len() <= remaining {
            self.response_text.push_str(text);
            return;
        }
        let retained = floor_char_boundary(text, remaining);
        self.response_text.push_str(&text[..retained]);
        observation.oversized_response_discarded_bytes = observation
            .oversized_response_discarded_bytes
            .saturating_add((text.len() - retained) as u64);
        self.materialize_partial_response(segment, observation);
        self.discarding_response_until_terminal = true;
    }

    fn capture_response_output_item(
        &mut self,
        segment: &PayloadSegment,
        value: &Value,
        observation: &mut ConnectionObservation,
    ) -> Result<(), ()> {
        let item = value.get("item").ok_or(())?;
        let item_bytes = serde_json::to_vec(item).map_err(|_| ())?.len();
        let Some(response_output_bytes) = self
            .response_output_bytes
            .checked_add(item_bytes)
            .filter(|bytes| {
                bytes.saturating_add(self.response_text.len()) <= self.max_response_bytes
            })
        else {
            observation.oversized_response_discarded_bytes = observation
                .oversized_response_discarded_bytes
                .saturating_add(item_bytes as u64);
            self.materialize_partial_response(segment, observation);
            self.discarding_response_until_terminal = true;
            return Ok(());
        };
        self.response_output.push(item.clone());
        self.response_output_bytes = response_output_bytes;
        Ok(())
    }

    fn materialize_partial_response(
        &mut self,
        source: &PayloadSegment,
        observation: &mut ConnectionObservation,
    ) {
        let Some(stream_key) = self.active_exchange_stream_key.take() else {
            self.clear_response();
            return;
        };
        let projected_before = observation.projected.len();
        self.append_partial_response_segments(source, &stream_key, observation);
        if observation.projected.len() > projected_before {
            observation.partial_exchange_streams.push(stream_key);
        } else {
            observation.completed_exchange_streams.push(stream_key);
        }
        self.clear_response();
    }

    fn append_partial_response_segments(
        &self,
        source: &PayloadSegment,
        stream_key: &PayloadStreamKey,
        observation: &mut ConnectionObservation,
    ) {
        let mut events = Vec::new();
        let max_text_chunk_bytes = (self.max_response_bytes / 8).min(16 * 1024);
        let mut offset = 0;
        while max_text_chunk_bytes > 0 && offset < self.response_text.len() {
            let relative_end = floor_char_boundary(
                &self.response_text[offset..],
                max_text_chunk_bytes.min(self.response_text.len() - offset),
            );
            if relative_end == 0 {
                observation.oversized_response_discarded_bytes = observation
                    .oversized_response_discarded_bytes
                    .saturating_add((self.response_text.len() - offset) as u64);
                break;
            }
            let text = &self.response_text[offset..offset + relative_end];
            let mut bytes = Vec::with_capacity(text.len().saturating_add(64));
            bytes.extend_from_slice(br#"data: {"type":"response.output_text.delta","delta":"#);
            if serde_json::to_writer(&mut bytes, text).is_err() {
                observation.oversized_response_discarded_bytes = observation
                    .oversized_response_discarded_bytes
                    .saturating_add(text.len() as u64);
                offset += relative_end;
                continue;
            }
            bytes.extend_from_slice(b"}\n\n");
            if bytes.len() <= self.max_response_bytes {
                events.push(bytes);
            } else {
                observation.oversized_response_discarded_bytes = observation
                    .oversized_response_discarded_bytes
                    .saturating_add(text.len() as u64);
            }
            offset += relative_end;
        }
        if max_text_chunk_bytes == 0 && !self.response_text.is_empty() {
            observation.oversized_response_discarded_bytes = observation
                .oversized_response_discarded_bytes
                .saturating_add(self.response_text.len() as u64);
        }
        for item in &self.response_output {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(br#"data: {"type":"response.output_item.done","item":"#);
            if serde_json::to_writer(&mut bytes, item).is_err() {
                continue;
            }
            bytes.extend_from_slice(b"}\n\n");
            if bytes.len() <= self.max_response_bytes {
                events.push(bytes);
            } else {
                observation.oversized_response_discarded_bytes = observation
                    .oversized_response_discarded_bytes
                    .saturating_add(bytes.len() as u64);
            }
        }
        if events.is_empty() {
            return;
        }
        observation.projected.push(self.synthetic_segment(
            source,
            stream_key.clone(),
            PayloadDirection::Inbound,
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n".to_vec(),
        ));
        observation
            .projected
            .extend(events.into_iter().map(|bytes| {
                let mut segment = self.synthetic_segment(
                    source,
                    stream_key.clone(),
                    PayloadDirection::Inbound,
                    bytes,
                );
                if let Some(started_at) = self.response_started_at {
                    segment.observed_at = started_at;
                }
                segment
            }));
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
        direction: PayloadDirection,
        bytes: Vec<u8>,
    ) -> PayloadSegment {
        let size = bytes.len() as u64;
        PayloadSegment {
            segment_id: source.segment_id,
            trace_id: source.trace_id,
            observed_at: source.observed_at,
            process: source.process,
            source_boundary: source.source_boundary,
            content_state: source.content_state,
            direction,
            stream_key,
            sequence: source.sequence,
            original_size: size,
            captured_size: size,
            operation_id: source.operation_id,
            operation_offset: 0,
            operation_original_size: size,
            operation_captured_size: size,
            operation_completion_state: source.operation_completion_state,
            truncation: PayloadTruncationState::Complete,
            redaction: source.redaction,
            library: "websocket".to_string(),
            symbol: "message".to_string(),
            protocol_hint: Some("websocket.responses".to_string()),
            bytes,
        }
    }
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn is_terminal_response_message(message_type: &str) -> bool {
    matches!(
        message_type,
        "response.completed" | "response.failed" | "response.incomplete"
    )
}
