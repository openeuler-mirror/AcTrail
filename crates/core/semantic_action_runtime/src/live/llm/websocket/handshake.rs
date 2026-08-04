//! HTTP upgrade recognition for WebSocket connections.

use model_core::payload::{PayloadSegment, PayloadStreamKey};

const HEADER_END: &[u8] = b"\r\n\r\n";
pub(super) const REQUEST_PREFIX: &[u8] = b"GET ";
pub(super) const ACCEPT_PREFIX: &[u8] = b"HTTP/1.1 101 ";
const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct HandshakeCandidate {
    buffer: Vec<u8>,
    stream_key: Option<PayloadStreamKey>,
    operation_id: Option<u64>,
    next_offset: u64,
    last_sequence: Option<u64>,
    scan_from: usize,
    header_end: Option<usize>,
}

impl HandshakeCandidate {
    pub(super) fn observe(&mut self, segment: &PayloadSegment, prefix: &[u8]) -> bool {
        if segment.operation_offset == 0 && segment.bytes.starts_with(prefix) {
            self.start(segment);
        } else if !self.accepts_continuation(segment) {
            return false;
        }
        if self.buffer.len().saturating_add(segment.bytes.len()) > MAX_HANDSHAKE_BYTES {
            self.reject(segment, "header_size_limit_exceeded");
            return true;
        }
        let Some(next_offset) = u64::try_from(segment.bytes.len())
            .ok()
            .and_then(|length| segment.operation_offset.checked_add(length))
        else {
            self.reject(segment, "operation_offset_overflow");
            return true;
        };
        self.buffer.extend_from_slice(&segment.bytes);
        self.operation_id = Some(segment.operation_id);
        self.next_offset = next_offset;
        self.last_sequence = Some(segment.sequence);
        self.scan_for_header_end();
        true
    }

    pub(super) fn request_path(&mut self) -> Option<String> {
        let header_end = self.header_end?;
        let path = Self::websocket_request_path(&self.buffer[..header_end]);
        self.clear();
        path
    }

    pub(super) fn accepted_extensions(&mut self) -> Option<NegotiatedExtensions> {
        let header_end = self.header_end?;
        let extensions = Self::websocket_accept(&self.buffer[..header_end]);
        self.clear();
        extensions
    }

    fn start(&mut self, segment: &PayloadSegment) {
        self.clear();
        self.stream_key = Some(segment.stream_key.clone());
    }

    fn accepts_continuation(&mut self, segment: &PayloadSegment) -> bool {
        if self.stream_key.as_ref() != Some(&segment.stream_key) {
            return false;
        }
        if self
            .last_sequence
            .is_some_and(|sequence| segment.sequence <= sequence)
        {
            self.reject(segment, "non_increasing_sequence");
            return false;
        }
        if self.operation_id == Some(segment.operation_id) {
            if segment.operation_offset != self.next_offset {
                self.reject(segment, "non_contiguous_operation_offset");
                return false;
            }
        } else if segment.operation_offset != 0 {
            self.reject(segment, "new_operation_with_nonzero_offset");
            return false;
        }
        // A TLS capture operation is one Rustls call, not a protocol-stream boundary.
        // Offset zero therefore starts either the candidate or its next operation.
        true
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

    fn reject(&mut self, segment: &PayloadSegment, reason: &'static str) {
        tracing::warn!(
            target: "actrail::semantic_projection",
            trace_id = %segment.trace_id,
            process_id = %segment.process.get(),
            direction = ?segment.direction,
            stream_key = %segment.stream_key,
            operation_id = segment.operation_id,
            operation_offset = segment.operation_offset,
            buffered_bytes = self.buffer.len(),
            reason,
            "discarded incomplete WebSocket handshake"
        );
        self.clear();
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
        if parts.next()? != "HTTP/1.1" || !Self::has_upgrade_headers(lines) {
            return None;
        }
        Some(path)
    }

    fn websocket_accept(bytes: &[u8]) -> Option<NegotiatedExtensions> {
        let header = std::str::from_utf8(bytes).ok()?;
        let mut lines = header.lines();
        if !lines.next()?.starts_with("HTTP/1.1 101 ") || !Self::has_upgrade_headers(lines.clone())
        {
            return None;
        }
        let extensions = Self::header_value(lines, "sec-websocket-extensions").unwrap_or_default();
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

    fn clear(&mut self) {
        self.buffer.clear();
        self.stream_key = None;
        self.operation_id = None;
        self.next_offset = 0;
        self.last_sequence = None;
        self.scan_from = 0;
        self.header_end = None;
    }
}

pub(super) struct NegotiatedExtensions {
    pub(super) permessage_deflate: bool,
    pub(super) client_no_context_takeover: bool,
    pub(super) server_no_context_takeover: bool,
}
