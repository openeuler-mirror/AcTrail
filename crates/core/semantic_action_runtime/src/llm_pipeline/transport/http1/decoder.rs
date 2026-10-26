//! Stateful HTTP/1 wire decoder.

use super::content_decoder::{DecoderFactory, DecoderHandle};
use std::ops::Range;
use std::sync::Arc;

const HEADER_END: &[u8] = b"\r\n\r\n";
const LINE_END: &[u8] = b"\r\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) enum Http1Direction {
    Request,
    Response,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) enum Http1DecodeFailure {
    BufferCapacity,
    InvalidContentLength,
    InvalidContentEncoding,
    InvalidHead,
    InvalidChunkSize,
    InvalidChunkTerminator,
}

#[derive(Clone, Debug)]
pub(in crate::llm_pipeline) enum DecodedHttp1Body {
    Shared(Arc<Vec<u8>>),
    WireRange(Range<usize>),
}

impl DecodedHttp1Body {
    pub(in crate::llm_pipeline) fn len(&self) -> usize {
        match self {
            Self::Shared(bytes) => bytes.len(),
            Self::WireRange(range) => range.len(),
        }
    }

    pub(in crate::llm_pipeline) fn bytes<'a>(&'a self, wire: &'a [u8]) -> Option<&'a [u8]> {
        match self {
            Self::Shared(bytes) => Some(bytes.as_slice()),
            Self::WireRange(range) => wire.get(range.clone()),
        }
    }

    pub(in crate::llm_pipeline) fn into_shared(self) -> Option<Arc<Vec<u8>>> {
        match self {
            Self::Shared(bytes) => Some(bytes),
            Self::WireRange(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::llm_pipeline) struct DecodedHttp1Message {
    pub(in crate::llm_pipeline) framing_failure: Option<Http1DecodeFailure>,
    pub(in crate::llm_pipeline) content_failure: Option<Http1DecodeFailure>,
    pub(in crate::llm_pipeline) message_id: Option<String>,
    pub(in crate::llm_pipeline) protocol: &'static str,
    pub(in crate::llm_pipeline) method: Option<String>,
    pub(in crate::llm_pipeline) authority: Option<String>,
    pub(in crate::llm_pipeline) path: Option<String>,
    pub(in crate::llm_pipeline) status_code: Option<String>,
    pub(in crate::llm_pipeline) reason: Option<String>,
    pub(in crate::llm_pipeline) headers_text: String,
    pub(in crate::llm_pipeline) body: DecodedHttp1Body,
    pub(in crate::llm_pipeline) declared_body_len: Option<usize>,
    pub(in crate::llm_pipeline) encoded_len: usize,
    pub(in crate::llm_pipeline) complete: bool,
    pub(in crate::llm_pipeline) body_boundary_known: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BodyFraming {
    None,
    Fixed(usize),
    Chunked,
    UntilEof,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChunkState {
    SizeLine { scan: usize },
    Data { remaining: usize },
    DataCrlf { matched: usize },
    Trailers { scan: usize },
    Complete,
}

struct ActiveMessage {
    protocol: &'static str,
    method: Option<String>,
    authority: Option<String>,
    path: Option<String>,
    status_code: Option<String>,
    reason: Option<String>,
    headers_text: String,
    body: Option<DecoderHandle>,
    content_failure: Option<Http1DecodeFailure>,
    max_decoded_bytes: usize,
    body_start: usize,
    wire_cursor: usize,
    framing: BodyFraming,
    chunk: ChunkState,
    complete: bool,
}

enum DecodeState {
    ReadingHead { scan: usize },
    ReadingBody(ActiveMessage),
    Failed(Http1DecodeFailure),
}

/// The caller owns connection routing and evidence. This decoder owns only
/// HTTP/1 framing cursors and decoded body bytes; every wire byte is scanned a
/// bounded number of times.
pub(in crate::llm_pipeline) struct Http1Decoder {
    direction: Http1Direction,
    max_buffer_bytes: usize,
    max_decoded_bytes: usize,
    state: DecodeState,
}

impl Http1Decoder {
    /// Confirms the message head before any body consumer runs. The exchange
    /// owner can supply request semantics (HEAD, CONNECT) at this boundary.
    pub(in crate::llm_pipeline) fn head(
        &mut self,
        wire: &[u8],
    ) -> Result<Option<DecodedHttp1Message>, Http1DecodeFailure> {
        if wire.len() > self.max_buffer_bytes {
            return Err(Http1DecodeFailure::BufferCapacity);
        }
        if matches!(self.state, DecodeState::ReadingHead { .. }) {
            self.advance_head(wire)?;
        }
        match &self.state {
            DecodeState::ReadingBody(active) => Ok(Some(active.snapshot())),
            DecodeState::ReadingHead { .. } => Ok(None),
            DecodeState::Failed(failure) => Err(*failure),
        }
    }

    pub(in crate::llm_pipeline) fn apply_request_method(&mut self, method: &str) -> bool {
        let DecodeState::ReadingBody(active) = &mut self.state else {
            return false;
        };
        let status = active
            .status_code
            .as_deref()
            .and_then(|value| value.parse::<u16>().ok());
        let tunnel = status == Some(101)
            || (method == "CONNECT" && status.is_some_and(|status| (200..300).contains(&status)));
        if method == "HEAD" || tunnel {
            active.framing = BodyFraming::None;
            active.complete = true;
            active.content_failure = None;
        }
        tunnel
    }

    pub(in crate::llm_pipeline) fn new(direction: Http1Direction, max_buffer_bytes: usize) -> Self {
        Self {
            direction,
            max_buffer_bytes,
            max_decoded_bytes: max_buffer_bytes,
            state: DecodeState::ReadingHead { scan: 0 },
        }
    }

    pub(in crate::llm_pipeline) fn with_decoded_limit(mut self, max_bytes: usize) -> Self {
        self.max_decoded_bytes = max_bytes;
        self
    }

    pub(in crate::llm_pipeline) fn reset(&mut self) {
        self.state = DecodeState::ReadingHead { scan: 0 };
    }

    pub(in crate::llm_pipeline) fn advance(
        &mut self,
        wire: &[u8],
        end_of_stream: bool,
    ) -> Result<Option<DecodedHttp1Message>, Http1DecodeFailure> {
        if wire.len() > self.max_buffer_bytes {
            self.state = DecodeState::Failed(Http1DecodeFailure::BufferCapacity);
        }
        if let DecodeState::Failed(failure) = self.state {
            return Err(failure);
        }
        if matches!(self.state, DecodeState::ReadingHead { .. }) {
            self.advance_head(wire)?;
        }
        let DecodeState::ReadingBody(active) = &mut self.state else {
            return Ok(None);
        };
        if let Err(failure) = Self::advance_body(active, wire, end_of_stream) {
            // Hand the successfully decoded prefix to the consumer before the
            // session aborts. The last provider event may be in this batch.
            let mut message = active.snapshot();
            message.framing_failure = Some(failure);
            self.state = DecodeState::Failed(failure);
            return Ok(Some(message));
        }
        Ok(Some(active.snapshot()))
    }

    fn advance_head(&mut self, wire: &[u8]) -> Result<(), Http1DecodeFailure> {
        let DecodeState::ReadingHead { scan } = &mut self.state else {
            return Ok(());
        };
        let search_start = scan.saturating_sub(HEADER_END.len() - 1);
        let Some(relative_end) = find_bytes(&wire[search_start..], HEADER_END) else {
            *scan = wire.len();
            return Ok(());
        };
        let header_end = search_start + relative_end;
        let body_start = header_end + HEADER_END.len();
        let headers_text = std::str::from_utf8(&wire[..header_end])
            .map_err(|_| Http1DecodeFailure::InvalidHead)?
            .to_string();
        let parsed = ParsedHead::parse(self.direction, &headers_text)?;
        let content = DecoderFactory::create(
            if parsed.framing == BodyFraming::None {
                ""
            } else {
                &headers_text
            },
            self.max_decoded_bytes,
        );
        let content_failure = content.as_ref().err().copied();
        let content = content.ok();
        self.state = DecodeState::ReadingBody(ActiveMessage {
            protocol: parsed.protocol,
            method: parsed.method,
            authority: parsed.authority,
            path: parsed.path,
            status_code: parsed.status_code,
            reason: parsed.reason,
            headers_text,
            // Fixed-length requests are projected synchronously from the
            // caller's wire buffer. Only decoded/chunked bodies and responses
            // need separate owned storage.
            body: if self.direction == Http1Direction::Request
                && matches!(parsed.framing, BodyFraming::Fixed(_))
                && content.as_ref().is_some_and(DecoderHandle::is_identity)
            {
                None
            } else {
                content
            },
            content_failure,
            max_decoded_bytes: self.max_decoded_bytes,
            body_start,
            wire_cursor: body_start,
            framing: parsed.framing,
            chunk: ChunkState::SizeLine { scan: body_start },
            complete: parsed.framing == BodyFraming::None,
        });
        Ok(())
    }

    fn advance_body(
        active: &mut ActiveMessage,
        wire: &[u8],
        end_of_stream: bool,
    ) -> Result<(), Http1DecodeFailure> {
        if active.complete {
            return Ok(());
        }
        match active.framing {
            BodyFraming::None => active.complete = true,
            BodyFraming::Fixed(expected) => {
                let expected_end = active
                    .body_start
                    .checked_add(expected)
                    .ok_or(Http1DecodeFailure::BufferCapacity)?;
                let available_end = wire.len().min(expected_end);
                active.copy_wire_delta(wire, available_end)?;
                active.complete = active.wire_cursor == expected_end;
            }
            BodyFraming::UntilEof => {
                active.copy_wire_delta(wire, wire.len())?;
                active.complete = end_of_stream;
            }
            BodyFraming::Chunked => Self::advance_chunked(active, wire)?,
        }
        if active.complete
            && let Some(body) = &mut active.body
        {
            if let Err(failure) = body.finish() {
                active.content_failure = Some(failure);
                active.body = None;
            }
        }
        Ok(())
    }

    fn advance_chunked(active: &mut ActiveMessage, wire: &[u8]) -> Result<(), Http1DecodeFailure> {
        loop {
            match active.chunk {
                ChunkState::SizeLine { scan } => {
                    let search_start = scan.saturating_sub(1).max(active.wire_cursor);
                    let Some(line_end) = find_bytes(&wire[search_start..], LINE_END) else {
                        active.chunk = ChunkState::SizeLine { scan: wire.len() };
                        return Ok(());
                    };
                    let line_end = search_start + line_end;
                    let size_text = std::str::from_utf8(&wire[active.wire_cursor..line_end])
                        .map_err(|_| Http1DecodeFailure::InvalidChunkSize)?
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim();
                    let size = match usize::from_str_radix(size_text, 16) {
                        Ok(size) => size,
                        Err(_)
                            if active.body.as_ref().is_none_or(DecoderHandle::is_identity)
                                && sse_field_line(&wire[active.wire_cursor..line_end]) =>
                        {
                            // Some L0 sources expose a de-chunked body while
                            // retaining the original Transfer-Encoding header.
                            // Switch only after one complete, decisive SSE
                            // field; an arbitrary malformed chunk stays local.
                            active.framing = BodyFraming::UntilEof;
                            active.wire_cursor = active.body_start;
                            if let Some(body) = &mut active.body {
                                body.clear_identity();
                            }
                            active.copy_wire_delta(wire, wire.len())?;
                            return Ok(());
                        }
                        Err(_) => return Err(Http1DecodeFailure::InvalidChunkSize),
                    };
                    active.wire_cursor = line_end + LINE_END.len();
                    active.chunk = if size == 0 {
                        ChunkState::Trailers {
                            scan: active.wire_cursor,
                        }
                    } else {
                        ChunkState::Data { remaining: size }
                    };
                }
                ChunkState::Data { remaining } => {
                    let available = wire.len().saturating_sub(active.wire_cursor);
                    let copied = available.min(remaining);
                    let end = active.wire_cursor + copied;
                    active.copy_wire_delta(wire, end)?;
                    let remaining = remaining - copied;
                    if remaining != 0 {
                        active.chunk = ChunkState::Data { remaining };
                        return Ok(());
                    }
                    active.chunk = ChunkState::DataCrlf { matched: 0 };
                }
                ChunkState::DataCrlf { mut matched } => {
                    while matched < LINE_END.len() && active.wire_cursor < wire.len() {
                        if wire[active.wire_cursor] != LINE_END[matched] {
                            return Err(Http1DecodeFailure::InvalidChunkTerminator);
                        }
                        active.wire_cursor += 1;
                        matched += 1;
                    }
                    if matched != LINE_END.len() {
                        active.chunk = ChunkState::DataCrlf { matched };
                        return Ok(());
                    }
                    active.chunk = ChunkState::SizeLine {
                        scan: active.wire_cursor,
                    };
                }
                ChunkState::Trailers { scan } => {
                    if wire.get(active.wire_cursor..active.wire_cursor + LINE_END.len())
                        == Some(LINE_END)
                    {
                        active.wire_cursor += LINE_END.len();
                        active.chunk = ChunkState::Complete;
                        active.complete = true;
                        return Ok(());
                    }
                    let search_start = scan.saturating_sub(1).max(active.wire_cursor);
                    let Some(line_end) = find_bytes(&wire[search_start..], LINE_END) else {
                        active.chunk = ChunkState::Trailers { scan: wire.len() };
                        return Ok(());
                    };
                    active.wire_cursor = search_start + line_end + LINE_END.len();
                    active.chunk = ChunkState::Trailers {
                        scan: active.wire_cursor,
                    };
                }
                ChunkState::Complete => return Ok(()),
            }
        }
    }
}

impl ActiveMessage {
    fn copy_wire_delta(&mut self, wire: &[u8], end: usize) -> Result<(), Http1DecodeFailure> {
        if end <= self.wire_cursor {
            return Ok(());
        }
        if self.body.is_none()
            && self.content_failure.is_none()
            && end.saturating_sub(self.body_start) > self.max_decoded_bytes
        {
            self.content_failure = Some(Http1DecodeFailure::BufferCapacity);
        }
        if let Some(body) = &mut self.body {
            if let Err(failure) = body.append(&wire[self.wire_cursor..end]) {
                self.content_failure = Some(failure);
                self.body = None;
            }
        }
        self.wire_cursor = end;
        Ok(())
    }

    fn snapshot(&self) -> DecodedHttp1Message {
        DecodedHttp1Message {
            framing_failure: None,
            content_failure: self.content_failure,
            message_id: None,
            protocol: self.protocol,
            method: self.method.clone(),
            authority: self.authority.clone(),
            path: self.path.clone(),
            status_code: self.status_code.clone(),
            reason: self.reason.clone(),
            headers_text: self.headers_text.clone(),
            body: match &self.body {
                Some(body) => DecodedHttp1Body::Shared(body.snapshot()),
                None if self.content_failure.is_some() => DecodedHttp1Body::WireRange(0..0),
                None => DecodedHttp1Body::WireRange(self.body_start..self.wire_cursor),
            },
            declared_body_len: match self.framing {
                BodyFraming::None => Some(0),
                BodyFraming::Fixed(length) => Some(length),
                BodyFraming::Chunked | BodyFraming::UntilEof => None,
            },
            encoded_len: self.wire_cursor,
            complete: self.complete,
            body_boundary_known: !matches!(self.framing, BodyFraming::UntilEof),
        }
    }
}

struct ParsedHead {
    protocol: &'static str,
    method: Option<String>,
    authority: Option<String>,
    path: Option<String>,
    status_code: Option<String>,
    reason: Option<String>,
    framing: BodyFraming,
}

impl ParsedHead {
    fn parse(direction: Http1Direction, headers_text: &str) -> Result<Self, Http1DecodeFailure> {
        let first_line = headers_text
            .split("\r\n")
            .next()
            .ok_or(Http1DecodeFailure::InvalidHead)?;
        let transfer_chunked = header_values(headers_text, "transfer-encoding")
            .last()
            .is_some_and(|value| {
                value
                    .split(',')
                    .next_back()
                    .is_some_and(|coding| coding.trim().eq_ignore_ascii_case("chunked"))
            });
        let content_length = content_length(headers_text)?;
        match direction {
            Http1Direction::Request => {
                let mut parts = first_line.split_whitespace();
                let method = parts.next().ok_or(Http1DecodeFailure::InvalidHead)?;
                let path = parts.next().ok_or(Http1DecodeFailure::InvalidHead)?;
                if !parts
                    .next()
                    .is_some_and(|version| version.starts_with("HTTP/"))
                {
                    return Err(Http1DecodeFailure::InvalidHead);
                }
                let authority = header_values(headers_text, "host")
                    .next()
                    .map(str::to_string);
                Ok(Self {
                    protocol: "http/1.1",
                    method: Some(method.to_string()),
                    authority,
                    path: Some(path.to_string()),
                    status_code: None,
                    reason: None,
                    framing: request_framing(transfer_chunked, content_length),
                })
            }
            Http1Direction::Response => {
                let mut parts = first_line.splitn(3, ' ');
                if !parts
                    .next()
                    .is_some_and(|version| version.starts_with("HTTP/"))
                {
                    return Err(Http1DecodeFailure::InvalidHead);
                }
                let status_code = parts
                    .next()
                    .filter(|status| {
                        status.len() == 3 && status.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    .ok_or(Http1DecodeFailure::InvalidHead)?;
                let no_body =
                    status_code.starts_with('1') || status_code == "204" || status_code == "304";
                Ok(Self {
                    protocol: "http/1.1",
                    method: None,
                    authority: None,
                    path: None,
                    status_code: Some(status_code.to_string()),
                    reason: parts.next().map(str::to_string),
                    framing: response_framing(transfer_chunked, content_length, no_body),
                })
            }
        }
    }
}

fn request_framing(chunked: bool, content_length: Option<usize>) -> BodyFraming {
    if chunked {
        BodyFraming::Chunked
    } else if let Some(length) = content_length {
        if length == 0 {
            BodyFraming::None
        } else {
            BodyFraming::Fixed(length)
        }
    } else {
        BodyFraming::None
    }
}

fn response_framing(chunked: bool, content_length: Option<usize>, no_body: bool) -> BodyFraming {
    if no_body {
        BodyFraming::None
    } else if chunked {
        BodyFraming::Chunked
    } else if let Some(length) = content_length {
        if length == 0 {
            BodyFraming::None
        } else {
            BodyFraming::Fixed(length)
        }
    } else {
        BodyFraming::UntilEof
    }
}

fn content_length(headers_text: &str) -> Result<Option<usize>, Http1DecodeFailure> {
    let mut parsed = None;
    for value in header_values(headers_text, "content-length") {
        for value in value.split(',') {
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| Http1DecodeFailure::InvalidContentLength)?;
            if parsed.is_some_and(|previous| previous != length) {
                return Err(Http1DecodeFailure::InvalidContentLength);
            }
            parsed = Some(length);
        }
    }
    Ok(parsed)
}

fn header_values<'a>(headers_text: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> {
    headers_text.split("\r\n").filter_map(move |line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn sse_field_line(line: &[u8]) -> bool {
    let start = line
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(line.len());
    let line = &line[start..];
    line.starts_with(b"data:")
        || line.starts_with(b"event:")
        || line.starts_with(b"id:")
        || line.starts_with(b"retry:")
        || line.starts_with(b":")
}
