//! Process-scoped WebSocket handshake and message assembly.

use std::collections::HashMap;

use crate::capture::{CaptureConfig, CaptureDirection, CaptureEvent};

use super::frame::FrameDecoder;
use super::message::MessageAssembler;
use super::model::{NegotiatedExtensions, WebSocketMessage};

const HEADER_END: &[u8] = b"\r\n\r\n";

#[derive(Debug)]
pub(crate) struct WebSocketAssembler {
    config: super::WebSocketConfig,
    processes: HashMap<u32, ProcessWebSocket>,
}

impl WebSocketAssembler {
    pub(crate) fn new(config: &CaptureConfig) -> Self {
        Self {
            config: config.websocket,
            processes: HashMap::new(),
        }
    }

    pub(crate) fn push(&mut self, event: &CaptureEvent) -> Vec<WebSocketMessage> {
        if event.flags.truncated {
            self.processes.remove(&event.pid);
            return Vec::new();
        }
        let process = self.processes.entry(event.pid).or_default();
        if event.direction == CaptureDirection::Outbound
            && let Some(path) = websocket_request_path(&event.captured)
        {
            process.offer(path);
            return Vec::new();
        }
        if event.direction == CaptureDirection::Inbound
            && let Some(extensions) = websocket_accept(&event.captured)
        {
            process.accept(extensions);
            return Vec::new();
        }
        process.push(event, self.config)
    }

    pub(crate) fn finish(&mut self) {
        self.processes.clear();
    }
}

#[derive(Debug, Default)]
struct ProcessWebSocket {
    offered_path: Option<String>,
    accepted: Option<NegotiatedExtensions>,
    connection: Option<WebSocketConnection>,
}

impl ProcessWebSocket {
    fn offer(&mut self, path: String) {
        self.offered_path = Some(path);
        self.accepted = None;
        self.connection = None;
    }

    fn accept(&mut self, extensions: NegotiatedExtensions) {
        if self.offered_path.is_none() {
            return;
        }
        self.accepted = Some(extensions);
        self.connection = None;
    }

    fn push(
        &mut self,
        event: &CaptureEvent,
        config: super::WebSocketConfig,
    ) -> Vec<WebSocketMessage> {
        if self.connection.is_none() {
            let expected_masked = event.direction == CaptureDirection::Outbound;
            if self.accepted.is_none()
                || !FrameDecoder::looks_like_frame(&event.captured, expected_masked)
            {
                return Vec::new();
            }
            let Some(path) = self.offered_path.clone() else {
                return Vec::new();
            };
            let Some(extensions) = self.accepted.clone() else {
                return Vec::new();
            };
            self.connection = Some(WebSocketConnection::new(
                event.stream_key,
                path,
                extensions,
                config,
            ));
        }
        let Some(connection) = &mut self.connection else {
            return Vec::new();
        };
        if connection.stream_key != event.stream_key {
            return Vec::new();
        }
        match connection.push(event) {
            Ok(messages) => messages,
            Err(()) => {
                self.connection = None;
                Vec::new()
            }
        }
    }
}

#[derive(Debug)]
struct WebSocketConnection {
    stream_key: u64,
    path: String,
    outbound: DirectionAssembler,
    inbound: DirectionAssembler,
}

impl WebSocketConnection {
    fn new(
        stream_key: u64,
        path: String,
        extensions: NegotiatedExtensions,
        config: super::WebSocketConfig,
    ) -> Self {
        Self {
            stream_key,
            path,
            outbound: DirectionAssembler::new(
                true,
                extensions.permessage_deflate,
                extensions.client_no_context_takeover,
                config,
            ),
            inbound: DirectionAssembler::new(
                false,
                extensions.permessage_deflate,
                extensions.server_no_context_takeover,
                config,
            ),
        }
    }

    fn push(&mut self, event: &CaptureEvent) -> Result<Vec<WebSocketMessage>, ()> {
        let assembler = match event.direction {
            CaptureDirection::Outbound => &mut self.outbound,
            CaptureDirection::Inbound => &mut self.inbound,
        };
        let messages = assembler.push(&event.captured).map_err(|_| ())?;
        Ok(messages
            .into_iter()
            .map(|message| WebSocketMessage {
                pid: event.pid,
                stream_key: event.stream_key,
                direction: event.direction,
                path: self.path.clone(),
                compressed: message.compressed,
                wire_bytes: message.wire_bytes,
                payload: message.payload,
            })
            .collect())
    }
}

#[derive(Debug)]
struct DirectionAssembler {
    frames: FrameDecoder,
    messages: MessageAssembler,
}

impl DirectionAssembler {
    fn new(
        expected_masked: bool,
        deflate_enabled: bool,
        no_context_takeover: bool,
        config: super::WebSocketConfig,
    ) -> Self {
        Self {
            frames: FrameDecoder::new(expected_masked, config.max_frame_buffer_bytes),
            messages: MessageAssembler::new(
                config.max_message_bytes,
                config.max_decoded_bytes,
                deflate_enabled,
                no_context_takeover,
            ),
        }
    }

    fn push(&mut self, bytes: &[u8]) -> crate::ToolResult<Vec<super::message::AssembledMessage>> {
        let mut messages = Vec::new();
        for frame in self.frames.push(bytes)? {
            if let Some(message) = self.messages.push(frame)? {
                messages.push(message);
            }
        }
        Ok(messages)
    }
}

fn websocket_request_path(bytes: &[u8]) -> Option<String> {
    let header = http_header(bytes)?;
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
    let header = http_header(bytes)?;
    let mut lines = header.lines();
    let first = lines.next()?;
    if !first.starts_with("HTTP/1.1 101 ") || !has_upgrade_headers(lines.clone()) {
        return None;
    }
    let extensions = header_value(lines, "sec-websocket-extensions").unwrap_or_default();
    let lower = extensions.to_ascii_lowercase();
    Some(NegotiatedExtensions {
        permessage_deflate: lower
            .split(',')
            .any(|extension| extension.trim().starts_with("permessage-deflate")),
        client_no_context_takeover: lower.contains("client_no_context_takeover"),
        server_no_context_takeover: lower.contains("server_no_context_takeover"),
    })
}

fn http_header(bytes: &[u8]) -> Option<&str> {
    let end = bytes
        .windows(HEADER_END.len())
        .position(|part| part == HEADER_END)?;
    std::str::from_utf8(&bytes[..end]).ok()
}

fn has_upgrade_headers<'a>(lines: impl Iterator<Item = &'a str>) -> bool {
    let mut upgrade = false;
    let mut connection = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("upgrade")
            && value.trim().eq_ignore_ascii_case("websocket")
        {
            upgrade = true;
        }
        if name.trim().eq_ignore_ascii_case("connection")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        {
            connection = true;
        }
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
