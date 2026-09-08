//! Responses WebSocket message projection onto the existing HTTP LLM seam.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::SystemTime;

use model_core::payload::{
    PayloadDirection, PayloadSegment, PayloadStreamKey, PayloadTruncationState,
};
use serde_json::Value;

use super::framing::DirectionAssembler;
use super::handshake::NegotiatedExtensions;

const MAX_PENDING_EXCHANGES: usize = 32;

pub(super) struct WebSocketConnection {
    stream_keys: BTreeSet<PayloadStreamKey>,
    observed_frame_stream: bool,
    synthetic_stream_key_prefix: String,
    next_exchange_id: u64,
    active_exchange_stream_key: Option<PayloadStreamKey>,
    pending_exchange_stream_keys: VecDeque<PayloadStreamKey>,
    path: String,
    outbound: DirectionAssembler,
    inbound: DirectionAssembler,
    response_text: String,
    response_output: Vec<Value>,
    response_custom_tool_inputs: BTreeMap<String, String>,
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
        let synthetic_stream_key_prefix =
            format!("websocket:{outbound_stream_key}:{inbound_stream_key}:exchange");
        Self {
            stream_keys: BTreeSet::from([outbound_stream_key, inbound_stream_key]),
            observed_frame_stream: false,
            synthetic_stream_key_prefix,
            next_exchange_id: 0,
            active_exchange_stream_key: None,
            pending_exchange_stream_keys: VecDeque::new(),
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
            response_custom_tool_inputs: BTreeMap::new(),
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
            .saturating_add(
                self.response_custom_tool_inputs
                    .values()
                    .map(String::len)
                    .sum(),
            )
            .saturating_add(self.response_output_bytes)
    }

    pub(super) fn is_bound_to(&self, stream_key: &PayloadStreamKey) -> bool {
        self.stream_keys.contains(stream_key)
    }

    pub(super) fn synthetic_stream_key_prefix(&self) -> &str {
        &self.synthetic_stream_key_prefix
    }

    fn accepts_segment(&mut self, segment: &PayloadSegment) -> bool {
        if self.stream_keys.contains(&segment.stream_key) {
            self.observed_frame_stream = true;
            return true;
        }
        let expected_masked = segment.direction == PayloadDirection::Outbound;
        if self.observed_frame_stream
            || !super::framing::FrameDecoder::looks_like_frame(&segment.bytes, expected_masked)
        {
            return false;
        }
        // TLS capture may assign the first data frame a different stream key
        // from the HTTP upgrade operation. Preserve that early binding while
        // retaining both handshake direction keys for subsequent routing.
        self.stream_keys.insert(segment.stream_key.clone());
        self.observed_frame_stream = true;
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
        let stream_key = PayloadStreamKey::new(format!(
            "{}:{}",
            self.synthetic_stream_key_prefix, self.next_exchange_id
        ));
        self.next_exchange_id = self.next_exchange_id.saturating_add(1);
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
            stream_key.clone(),
            PayloadDirection::Outbound,
            bytes,
        ));
        if self.active_exchange_stream_key.is_some() || self.discarding_response_until_terminal {
            if self.pending_exchange_stream_keys.len() < MAX_PENDING_EXCHANGES {
                self.pending_exchange_stream_keys.push_back(stream_key);
            } else {
                observation.completed_exchange_streams.push(stream_key);
                observation.superseded_responses =
                    observation.superseded_responses.saturating_add(1);
            }
            return;
        }
        self.discarding_response_until_terminal = false;
        self.clear_response();
        self.active_exchange_stream_key = Some(stream_key);
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
                    self.activate_next_exchange();
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
        if message_type == "response.custom_tool_call_input.delta" {
            self.capture_custom_tool_input_delta(value);
            return Ok(());
        }
        if message_type == "response.custom_tool_call_input.done" {
            self.capture_custom_tool_input_done(value);
            return Ok(());
        }
        let output_item = value.get("item");
        if message_type == "response.output_item.done"
            || (message_type == "response.output_item.added"
                && output_item.is_some_and(response_output_item_is_tool_call))
        {
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
            self.activate_next_exchange();
            return Ok(());
        };
        Self::ensure_response_output(&mut response, &self.response_output, &self.response_text);
        let Ok(body) = serde_json::to_vec(&response) else {
            observation.decode_failed_entries = observation.decode_failed_entries.saturating_add(1);
            observation.decode_discarded_bytes = observation
                .decode_discarded_bytes
                .saturating_add(message_bytes as u64);
            self.materialize_partial_response(segment, observation);
            self.activate_next_exchange();
            return Ok(());
        };
        let mut bytes = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(&body);
        let Some(stream_key) = self.active_exchange_stream_key.take() else {
            self.activate_next_exchange();
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
        self.activate_next_exchange();
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
        let mut item = value.get("item").cloned().ok_or(())?;
        self.merge_custom_tool_input(&mut item);
        let existing_index = self
            .response_output
            .iter()
            .position(|existing| response_output_items_match(existing, &item));
        if let Some(index) = existing_index {
            preserve_existing_output_item_fields(&self.response_output[index], &mut item);
        }
        let item_bytes = serde_json::to_vec(&item).map_err(|_| ())?.len();
        let existing_bytes = existing_index
            .and_then(|index| serde_json::to_vec(&self.response_output[index]).ok())
            .map_or(0, |item| item.len());
        let Some(response_output_bytes) = self
            .response_output_bytes
            .checked_sub(existing_bytes)
            .and_then(|bytes| bytes.checked_add(item_bytes))
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
        if let Some(index) = existing_index {
            self.response_output[index] = item;
        } else {
            self.response_output.push(item);
        }
        self.response_output_bytes = response_output_bytes;
        Ok(())
    }

    fn capture_custom_tool_input_delta(&mut self, value: &Value) {
        let Some(item_id) = custom_tool_item_id(value) else {
            return;
        };
        let Some(delta) = value.get("delta").and_then(Value::as_str) else {
            return;
        };
        let input = {
            let input = self
                .response_custom_tool_inputs
                .entry(item_id.to_string())
                .or_default();
            input.push_str(delta);
            input.clone()
        };
        self.update_captured_custom_tool_input(item_id, input);
    }

    fn capture_custom_tool_input_done(&mut self, value: &Value) {
        let Some(item_id) = custom_tool_item_id(value) else {
            return;
        };
        let Some(input) = value.get("input").and_then(Value::as_str) else {
            return;
        };
        self.response_custom_tool_inputs
            .insert(item_id.to_string(), input.to_string());
        self.update_captured_custom_tool_input(item_id, input.to_string());
    }

    fn merge_custom_tool_input(&mut self, item: &mut Value) {
        if item.get("type").and_then(Value::as_str) != Some("custom_tool_call") {
            return;
        }
        let item_ids = response_output_item_ids(item)
            .map(str::to_string)
            .collect::<Vec<_>>();
        if item_ids.is_empty() {
            return;
        }
        let input = item_ids
            .iter()
            .find_map(|item_id| self.response_custom_tool_inputs.remove(item_id))
            .or_else(|| {
                if self.response_custom_tool_inputs.len() != 1 {
                    return None;
                }
                let key = self.response_custom_tool_inputs.keys().next()?.clone();
                self.response_custom_tool_inputs.remove(&key)
            });
        let Some(input) = input else { return };
        if item
            .get("input")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            item["input"] = Value::String(input);
        }
    }

    fn update_captured_custom_tool_input(&mut self, item_id: &str, input: String) {
        let Some(item) = self.response_output.iter_mut().find(|item| {
            item.get("type").and_then(Value::as_str) == Some("custom_tool_call")
                && response_output_item_ids(item).any(|candidate| candidate == item_id)
        }) else {
            return;
        };
        item["input"] = Value::String(input);
        self.response_output_bytes = self
            .response_output
            .iter()
            .filter_map(|item| serde_json::to_vec(item).ok())
            .map(|item| item.len())
            .sum();
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
        self.append_partial_response_segments(source, &stream_key, observation);
        // Reaching this path with an active exchange means response.create
        // was already projected as an LLM request. Even when no response
        // bytes were observed, the synthetic stream must be finalized as
        // partial so the facade emits a request-only llm.call instead of
        // forgetting the open request as if the exchange had completed.
        observation.partial_exchange_streams.push(stream_key);
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

    fn activate_next_exchange(&mut self) {
        self.clear_response();
        self.active_exchange_stream_key = self.pending_exchange_stream_keys.pop_front();
    }

    fn clear_response(&mut self) {
        self.response_text.clear();
        self.response_output.clear();
        self.response_custom_tool_inputs.clear();
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

fn custom_tool_item_id(value: &Value) -> Option<&str> {
    ["item_id", "call_id", "id"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
}

fn response_output_item_is_tool_call(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some("function_call" | "custom_tool_call")
    )
}

fn response_output_item_ids(item: &Value) -> impl Iterator<Item = &str> {
    ["id", "call_id"]
        .into_iter()
        .filter_map(|key| item.get(key).and_then(Value::as_str))
        .filter(|id| !id.is_empty())
}

fn response_output_items_match(existing: &Value, incoming: &Value) -> bool {
    response_output_item_ids(existing)
        .any(|existing_id| response_output_item_ids(incoming).any(|id| id == existing_id))
        || existing == incoming
}

fn preserve_existing_output_item_fields(existing: &Value, incoming: &mut Value) {
    let (Some(existing), Some(incoming)) = (existing.as_object(), incoming.as_object_mut()) else {
        return;
    };
    for (key, value) in existing {
        let missing_or_empty = incoming
            .get(key)
            .is_none_or(|candidate| candidate.as_str().is_some_and(str::is_empty));
        if missing_or_empty {
            incoming.insert(key.clone(), value.clone());
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
