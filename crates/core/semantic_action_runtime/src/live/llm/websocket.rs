//! Incremental WebSocket decoding for LLM protocols transported after an HTTP upgrade.

use std::collections::BTreeMap;

use flate2::{Decompress, FlushDecompress, Status};
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadDirection, PayloadSegment, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use serde_json::Value;

const HEADER_END: &[u8] = b"\r\n\r\n";
const REQUEST_PREFIX: &[u8] = b"GET ";
const ACCEPT_PREFIX: &[u8] = b"HTTP/1.1 101 ";
const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;
const MAX_FRAME_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 32 * 1024 * 1024;
const COMPACT_AFTER_BYTES: usize = 64 * 1024;
const DEFLATE_TAIL: &[u8] = &[0x00, 0x00, 0xff, 0xff];

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProcessKey {
    trace_id: TraceId,
    process: ProcessIdentity,
}

#[derive(Default)]
pub(super) struct WebSocketLlmAdapter {
    processes: BTreeMap<ProcessKey, ProcessWebSocket>,
}

impl WebSocketLlmAdapter {
    pub(super) fn observe(&mut self, segment: &PayloadSegment) -> Vec<PayloadSegment> {
        if segment.source_boundary != PayloadSourceBoundary::TlsUserSpace {
            return Vec::new();
        }
        let key = ProcessKey {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
        };
        if segment.truncation == PayloadTruncationState::Truncated {
            self.processes.remove(&key);
            return Vec::new();
        }
        self.processes.entry(key).or_default().observe(segment)
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.processes.retain(|key, _| key.trace_id != trace_id);
    }
}

#[derive(Default)]
struct ProcessWebSocket {
    outbound_handshake: HandshakeCandidate,
    inbound_handshake: HandshakeCandidate,
    pending_offer: Option<String>,
    accepted: Option<AcceptedHandshake>,
    connection: Option<WebSocketConnection>,
}

impl ProcessWebSocket {
    fn observe(&mut self, segment: &PayloadSegment) -> Vec<PayloadSegment> {
        if self.observe_handshake(segment) {
            return Vec::new();
        }
        if self.connection.is_none() {
            let expected_masked = segment.direction == PayloadDirection::Outbound;
            if self.accepted.is_some()
                && FrameDecoder::looks_like_frame(&segment.bytes, expected_masked)
            {
                let Some(accepted) = self.accepted.take() else {
                    return Vec::new();
                };
                self.connection = Some(WebSocketConnection::new(
                    segment.stream_key.clone(),
                    accepted.path,
                    accepted.extensions,
                ));
            }
        }
        let Some(connection) = self.connection.as_mut() else {
            return Vec::new();
        };
        if connection.stream_key != segment.stream_key {
            return Vec::new();
        }
        match connection.observe(segment) {
            Ok(observation) => {
                if observation.closed {
                    self.connection = None;
                }
                observation.projected
            }
            Err(()) => {
                self.connection = None;
                Vec::new()
            }
        }
    }

    fn observe_handshake(&mut self, segment: &PayloadSegment) -> bool {
        match segment.direction {
            PayloadDirection::Outbound => {
                let observed = self.outbound_handshake.observe(segment, REQUEST_PREFIX);
                if let Some(path) = self.outbound_handshake.request_path() {
                    self.pending_offer = Some(path);
                    self.accepted = None;
                }
                observed
            }
            PayloadDirection::Inbound => {
                let observed = self.inbound_handshake.observe(segment, ACCEPT_PREFIX);
                if let Some(extensions) = self.inbound_handshake.accepted_extensions()
                    && let Some(path) = self.pending_offer.take()
                {
                    self.accepted = Some(AcceptedHandshake { path, extensions });
                    self.connection = None;
                }
                observed
            }
        }
    }
}

#[derive(Default)]
struct HandshakeCandidate {
    buffer: Vec<u8>,
    operation_id: Option<u64>,
    next_offset: u64,
    scan_from: usize,
    header_end: Option<usize>,
}

impl HandshakeCandidate {
    fn observe(&mut self, segment: &PayloadSegment, prefix: &[u8]) -> bool {
        if segment.operation_offset == 0 {
            if !segment.bytes.starts_with(prefix) {
                if self.operation_id.is_some() {
                    self.clear();
                }
                return false;
            }
            self.clear();
            self.operation_id = Some(segment.operation_id);
        } else if self.operation_id != Some(segment.operation_id)
            || segment.operation_offset != self.next_offset
        {
            if self.operation_id.is_some() {
                self.clear();
            }
            return false;
        }
        if self.buffer.len().saturating_add(segment.bytes.len()) > MAX_HANDSHAKE_BYTES {
            self.clear();
            return true;
        }
        let Some(next_offset) = u64::try_from(segment.bytes.len())
            .ok()
            .and_then(|length| segment.operation_offset.checked_add(length))
        else {
            self.clear();
            return true;
        };
        self.buffer.extend_from_slice(&segment.bytes);
        self.next_offset = next_offset;
        self.scan_for_header_end();
        true
    }

    fn request_path(&mut self) -> Option<String> {
        let header_end = self.header_end?;
        let path = websocket_request_path(&self.buffer[..header_end]);
        self.clear();
        path
    }

    fn accepted_extensions(&mut self) -> Option<NegotiatedExtensions> {
        let header_end = self.header_end?;
        let extensions = websocket_accept(&self.buffer[..header_end]);
        self.clear();
        extensions
    }

    fn scan_for_header_end(&mut self) {
        let Some(relative_end) = self.buffer[self.scan_from..]
            .windows(HEADER_END.len())
            .position(|part| part == HEADER_END)
        else {
            self.scan_from = self
                .buffer
                .len()
                .saturating_sub(HEADER_END.len().saturating_sub(1));
            return;
        };
        self.header_end = Some(self.scan_from + relative_end);
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.operation_id = None;
        self.next_offset = 0;
        self.scan_from = 0;
        self.header_end = None;
    }
}

struct AcceptedHandshake {
    path: String,
    extensions: NegotiatedExtensions,
}

struct NegotiatedExtensions {
    permessage_deflate: bool,
    client_no_context_takeover: bool,
    server_no_context_takeover: bool,
}

struct WebSocketConnection {
    stream_key: PayloadStreamKey,
    synthetic_stream_key: PayloadStreamKey,
    path: String,
    outbound: DirectionAssembler,
    inbound: DirectionAssembler,
    response_text: String,
}

struct ConnectionObservation {
    projected: Vec<PayloadSegment>,
    closed: bool,
}

impl WebSocketConnection {
    fn new(stream_key: PayloadStreamKey, path: String, extensions: NegotiatedExtensions) -> Self {
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

    fn observe(&mut self, segment: &PayloadSegment) -> Result<ConnectionObservation, ()> {
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
                    if value.get("type").and_then(Value::as_str) != Some("response.create") {
                        continue;
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
                    projected.push(synthetic_segment(
                        segment,
                        self.synthetic_stream_key.clone(),
                        bytes,
                    ));
                }
                PayloadDirection::Inbound => {
                    let Some(message_type) = value.get("type").and_then(Value::as_str) else {
                        continue;
                    };
                    if !message_type.starts_with("response.") {
                        continue;
                    }
                    if message_type == "response.output_text.delta" {
                        if let Some(delta) = value.get("delta").and_then(Value::as_str)
                            && self.response_text.len().saturating_add(delta.len())
                                <= MAX_DECODED_BYTES
                        {
                            self.response_text.push_str(delta);
                        }
                        continue;
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
                        continue;
                    }
                    let Some(mut response) = value.get("response").cloned() else {
                        self.response_text.clear();
                        continue;
                    };
                    ensure_response_output(&mut response, &self.response_text);
                    let Ok(body) = serde_json::to_vec(&response) else {
                        self.response_text.clear();
                        continue;
                    };
                    self.response_text.clear();
                    let mut bytes = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    bytes.extend_from_slice(&body);
                    projected.push(synthetic_segment(
                        segment,
                        self.synthetic_stream_key.clone(),
                        bytes,
                    ));
                }
            }
        }
        Ok(ConnectionObservation {
            projected,
            closed: assembled.closed,
        })
    }
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

fn synthetic_segment(
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

struct DirectionAssembler {
    frames: FrameDecoder,
    message: MessageAssembler,
}

struct DirectionObservation {
    messages: Vec<Vec<u8>>,
    closed: bool,
}

impl DirectionAssembler {
    fn new(expected_masked: bool, deflate_enabled: bool, no_context_takeover: bool) -> Self {
        Self {
            frames: FrameDecoder::new(expected_masked),
            message: MessageAssembler::new(deflate_enabled, no_context_takeover),
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<DirectionObservation, ()> {
        let mut messages = Vec::new();
        let mut closed = false;
        for frame in self.frames.push(bytes)? {
            closed |= matches!(frame.opcode, Opcode::Close);
            if let Some(message) = self.message.push(frame)? {
                messages.push(message);
            }
        }
        Ok(DirectionObservation { messages, closed })
    }
}

#[derive(Clone, Copy)]
enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl Opcode {
    fn parse(value: u8) -> Option<Self> {
        match value {
            0x0 => Some(Self::Continuation),
            0x1 => Some(Self::Text),
            0x2 => Some(Self::Binary),
            0x8 => Some(Self::Close),
            0x9 => Some(Self::Ping),
            0xa => Some(Self::Pong),
            _ => None,
        }
    }

    fn is_control(self) -> bool {
        matches!(self, Self::Close | Self::Ping | Self::Pong)
    }
}

struct WebSocketFrame {
    fin: bool,
    compressed: bool,
    opcode: Opcode,
    payload: Vec<u8>,
}

struct FrameDecoder {
    expected_masked: bool,
    buffer: Vec<u8>,
    cursor: usize,
}

impl FrameDecoder {
    fn new(expected_masked: bool) -> Self {
        Self {
            expected_masked,
            buffer: Vec::new(),
            cursor: 0,
        }
    }

    fn looks_like_frame(bytes: &[u8], expected_masked: bool) -> bool {
        let Some((&first, rest)) = bytes.split_first() else {
            return false;
        };
        let Some(&second) = rest.first() else {
            return false;
        };
        first & 0x30 == 0
            && Opcode::parse(first & 0x0f).is_some()
            && (second & 0x80 != 0) == expected_masked
    }

    fn push(&mut self, bytes: &[u8]) -> Result<Vec<WebSocketFrame>, ()> {
        if self
            .buffer
            .len()
            .saturating_sub(self.cursor)
            .saturating_add(bytes.len())
            > MAX_FRAME_BUFFER_BYTES
        {
            self.clear();
            return Err(());
        }
        self.compact_if_needed();
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some(frame) = self.take_frame()? {
            frames.push(frame);
        }
        self.compact_if_needed();
        Ok(frames)
    }

    fn take_frame(&mut self) -> Result<Option<WebSocketFrame>, ()> {
        let bytes = &self.buffer[self.cursor..];
        if bytes.len() < 2 {
            return Ok(None);
        }
        let first = bytes[0];
        let second = bytes[1];
        let Some(opcode) = Opcode::parse(first & 0x0f) else {
            return self.fail();
        };
        let fin = first & 0x80 != 0;
        let compressed = first & 0x40 != 0;
        let masked = second & 0x80 != 0;
        if first & 0x30 != 0
            || masked != self.expected_masked
            || (opcode.is_control() && (!fin || compressed))
        {
            return self.fail();
        }
        let mut header_len = 2usize;
        let mut payload_len = usize::from(second & 0x7f);
        if payload_len == 126 {
            if bytes.len() < 4 {
                return Ok(None);
            }
            payload_len = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
            header_len = 4;
        } else if payload_len == 127 {
            if bytes.len() < 10 {
                return Ok(None);
            }
            let encoded = u64::from_be_bytes(bytes[2..10].try_into().map_err(|_| ())?);
            if encoded & (1 << 63) != 0 {
                return self.fail();
            }
            payload_len = usize::try_from(encoded).map_err(|_| ())?;
            header_len = 10;
        }
        if opcode.is_control() && payload_len > 125 {
            return self.fail();
        }
        let mask = if masked {
            if bytes.len() < header_len + 4 {
                return Ok(None);
            }
            let mask: [u8; 4] = bytes[header_len..header_len + 4]
                .try_into()
                .map_err(|_| ())?;
            header_len += 4;
            Some(mask)
        } else {
            None
        };
        let frame_len = header_len.checked_add(payload_len).ok_or(())?;
        if frame_len > MAX_FRAME_BUFFER_BYTES {
            return self.fail();
        }
        if bytes.len() < frame_len {
            return Ok(None);
        }
        let mut payload = bytes[header_len..frame_len].to_vec();
        if let Some(mask) = mask {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        self.cursor += frame_len;
        Ok(Some(WebSocketFrame {
            fin,
            compressed,
            opcode,
            payload,
        }))
    }

    fn compact_if_needed(&mut self) {
        if self.cursor == self.buffer.len() {
            self.clear();
        } else if self.cursor >= COMPACT_AFTER_BYTES && self.cursor >= self.buffer.len() / 2 {
            self.buffer.copy_within(self.cursor.., 0);
            self.buffer.truncate(self.buffer.len() - self.cursor);
            self.cursor = 0;
        }
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    fn fail<T>(&mut self) -> Result<T, ()> {
        self.clear();
        Err(())
    }
}

struct FragmentedMessage {
    opcode: Opcode,
    compressed: bool,
    payload: Vec<u8>,
}

struct MessageAssembler {
    fragment: Option<FragmentedMessage>,
    deflate: PerMessageDeflateDecoder,
}

impl MessageAssembler {
    fn new(deflate_enabled: bool, no_context_takeover: bool) -> Self {
        Self {
            fragment: None,
            deflate: PerMessageDeflateDecoder::new(deflate_enabled, no_context_takeover),
        }
    }

    fn push(&mut self, frame: WebSocketFrame) -> Result<Option<Vec<u8>>, ()> {
        if frame.opcode.is_control() {
            return Ok(None);
        }
        match frame.opcode {
            Opcode::Text | Opcode::Binary => {
                if self.fragment.is_some() || frame.payload.len() > MAX_MESSAGE_BYTES {
                    return self.fail();
                }
                if frame.fin {
                    return self.complete(frame.compressed, frame.payload).map(Some);
                }
                self.fragment = Some(FragmentedMessage {
                    opcode: frame.opcode,
                    compressed: frame.compressed,
                    payload: frame.payload,
                });
                Ok(None)
            }
            Opcode::Continuation => {
                if frame.compressed {
                    return self.fail();
                }
                let Some(mut fragment) = self.fragment.take() else {
                    return self.fail();
                };
                if fragment.payload.len().saturating_add(frame.payload.len()) > MAX_MESSAGE_BYTES {
                    return self.fail();
                }
                fragment.payload.extend_from_slice(&frame.payload);
                if !frame.fin {
                    self.fragment = Some(fragment);
                    return Ok(None);
                }
                let _opcode = fragment.opcode;
                self.complete(fragment.compressed, fragment.payload)
                    .map(Some)
            }
            Opcode::Close | Opcode::Ping | Opcode::Pong => Ok(None),
        }
    }

    fn complete(&mut self, compressed: bool, payload: Vec<u8>) -> Result<Vec<u8>, ()> {
        if compressed {
            self.deflate.decode(&payload)
        } else {
            Ok(payload)
        }
    }

    fn fail<T>(&mut self) -> Result<T, ()> {
        self.fragment = None;
        Err(())
    }
}

struct PerMessageDeflateDecoder {
    enabled: bool,
    no_context_takeover: bool,
    decoder: Decompress,
}

impl PerMessageDeflateDecoder {
    fn new(enabled: bool, no_context_takeover: bool) -> Self {
        Self {
            enabled,
            no_context_takeover,
            decoder: Decompress::new(false),
        }
    }

    fn decode(&mut self, payload: &[u8]) -> Result<Vec<u8>, ()> {
        if !self.enabled {
            return Err(());
        }
        let mut input = Vec::with_capacity(payload.len() + DEFLATE_TAIL.len());
        input.extend_from_slice(payload);
        input.extend_from_slice(DEFLATE_TAIL);
        let mut cursor = 0usize;
        let mut output = Vec::new();
        loop {
            let mut chunk = [0u8; 4096];
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            let status = self
                .decoder
                .decompress(&input[cursor..], &mut chunk, FlushDecompress::Sync)
                .map_err(|_| ())?;
            let consumed = usize::try_from(self.decoder.total_in() - before_in).map_err(|_| ())?;
            let produced =
                usize::try_from(self.decoder.total_out() - before_out).map_err(|_| ())?;
            cursor = cursor.checked_add(consumed).ok_or(())?;
            if output.len().saturating_add(produced) > MAX_DECODED_BYTES {
                return Err(());
            }
            output.extend_from_slice(&chunk[..produced]);
            if cursor == input.len() && produced < chunk.len() {
                break;
            }
            if consumed == 0 && produced == 0 {
                if cursor == input.len() && status == Status::BufError {
                    break;
                }
                return Err(());
            }
        }
        if cursor != input.len() {
            return Err(());
        }
        if self.no_context_takeover {
            self.decoder.reset(false);
        }
        Ok(output)
    }
}

fn websocket_request_path(bytes: &[u8]) -> Option<String> {
    let header = std::str::from_utf8(bytes).ok()?;
    let mut lines = header.lines();
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let path = parts.next()?.to_string();
    if parts.next()? != "HTTP/1.1" || !has_upgrade_headers(lines) {
        return None;
    }
    Some(path)
}

fn websocket_accept(bytes: &[u8]) -> Option<NegotiatedExtensions> {
    let header = std::str::from_utf8(bytes).ok()?;
    let mut lines = header.lines();
    if !lines.next()?.starts_with("HTTP/1.1 101 ") || !has_upgrade_headers(lines.clone()) {
        return None;
    }
    let extensions = header_value(lines, "sec-websocket-extensions").unwrap_or_default();
    let lower = extensions.to_ascii_lowercase();
    Some(NegotiatedExtensions {
        permessage_deflate: lower
            .split(',')
            .any(|value| value.trim().starts_with("permessage-deflate")),
        client_no_context_takeover: lower.contains("client_no_context_takeover"),
        server_no_context_takeover: lower.contains("server_no_context_takeover"),
    })
}

fn has_upgrade_headers<'a>(lines: impl Iterator<Item = &'a str>) -> bool {
    let mut upgrade = false;
    let mut connection = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        upgrade |= name.trim().eq_ignore_ascii_case("upgrade")
            && value.trim().eq_ignore_ascii_case("websocket");
        connection |= name.trim().eq_ignore_ascii_case("connection")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"));
    }
    upgrade && connection
}

fn header_value<'a>(lines: impl Iterator<Item = &'a str>, expected: &str) -> Option<&'a str> {
    lines
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            name.trim()
                .eq_ignore_ascii_case(expected)
                .then_some(value.trim())
        })
}
