//! Stateful HTTP/1 wire decoder.

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
    InvalidHead,
    InvalidChunkSize,
    InvalidChunkTerminator,
}

impl Http1DecodeFailure {
    pub(in crate::llm_pipeline) const fn as_str(self) -> &'static str {
        match self {
            Self::BufferCapacity => "buffer_capacity",
            Self::InvalidContentLength => "invalid_content_length",
            Self::InvalidHead => "invalid_head",
            Self::InvalidChunkSize => "invalid_chunk_size",
            Self::InvalidChunkTerminator => "invalid_chunk_terminator",
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::llm_pipeline) struct DecodedHttp1Message {
    pub(in crate::llm_pipeline) protocol: &'static str,
    pub(in crate::llm_pipeline) method: Option<String>,
    pub(in crate::llm_pipeline) authority: Option<String>,
    pub(in crate::llm_pipeline) path: Option<String>,
    pub(in crate::llm_pipeline) status_code: Option<String>,
    pub(in crate::llm_pipeline) reason: Option<String>,
    pub(in crate::llm_pipeline) headers_text: String,
    pub(in crate::llm_pipeline) body: Arc<Vec<u8>>,
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
    body: Arc<Vec<u8>>,
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
    state: DecodeState,
}

impl Http1Decoder {
    pub(in crate::llm_pipeline) fn new(direction: Http1Direction, max_buffer_bytes: usize) -> Self {
        Self {
            direction,
            max_buffer_bytes,
            state: DecodeState::ReadingHead { scan: 0 },
        }
    }

    pub(in crate::llm_pipeline) fn new_raw_chunked_response(max_buffer_bytes: usize) -> Self {
        Self {
            direction: Http1Direction::Response,
            max_buffer_bytes,
            state: DecodeState::ReadingBody(ActiveMessage {
                protocol: "http/1.1",
                method: None,
                authority: None,
                path: None,
                status_code: None,
                reason: None,
                headers_text: String::new(),
                body: Arc::new(Vec::new()),
                body_start: 0,
                wire_cursor: 0,
                framing: BodyFraming::Chunked,
                chunk: ChunkState::SizeLine { scan: 0 },
                complete: false,
            }),
        }
    }

    pub(in crate::llm_pipeline) fn reset(&mut self) {
        self.state = DecodeState::ReadingHead { scan: 0 };
    }

    pub(in crate::llm_pipeline) fn snapshot(&self) -> Option<DecodedHttp1Message> {
        match &self.state {
            DecodeState::ReadingBody(active) => Some(active.snapshot()),
            DecodeState::ReadingHead { .. } | DecodeState::Failed(_) => None,
        }
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
        Self::advance_body(active, wire, end_of_stream)?;
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
        self.state = DecodeState::ReadingBody(ActiveMessage {
            protocol: parsed.protocol,
            method: parsed.method,
            authority: parsed.authority,
            path: parsed.path,
            status_code: parsed.status_code,
            reason: parsed.reason,
            headers_text,
            body: Arc::new(Vec::new()),
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
                active.copy_wire_delta(wire, available_end);
                active.complete = active.wire_cursor == expected_end;
            }
            BodyFraming::UntilEof => {
                active.copy_wire_delta(wire, wire.len());
                active.complete = end_of_stream;
            }
            BodyFraming::Chunked => Self::advance_chunked(active, wire)?,
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
                        Err(_) if sse_field_line(&wire[active.wire_cursor..line_end]) => {
                            // Some L0 sources expose a de-chunked body while
                            // retaining the original Transfer-Encoding header.
                            // Switch only after one complete, decisive SSE
                            // field; an arbitrary malformed chunk stays local.
                            active.framing = BodyFraming::UntilEof;
                            active.wire_cursor = active.body_start;
                            Arc::make_mut(&mut active.body).clear();
                            active.copy_wire_delta(wire, wire.len());
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
                    active.copy_wire_delta(wire, end);
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

pub(in crate::llm_pipeline) fn raw_chunked_candidate(bytes: &[u8]) -> bool {
    let Some(line_end) = find_bytes(bytes, LINE_END) else {
        return false;
    };
    let Ok(line) = std::str::from_utf8(&bytes[..line_end]) else {
        return false;
    };
    let size_text = line.split(';').next().unwrap_or_default().trim();
    !size_text.is_empty() && usize::from_str_radix(size_text, 16).is_ok()
}

impl ActiveMessage {
    fn copy_wire_delta(&mut self, wire: &[u8], end: usize) {
        if end <= self.wire_cursor {
            return;
        }
        Arc::make_mut(&mut self.body).extend_from_slice(&wire[self.wire_cursor..end]);
        self.wire_cursor = end;
    }

    fn snapshot(&self) -> DecodedHttp1Message {
        DecodedHttp1Message {
            protocol: self.protocol,
            method: self.method.clone(),
            authority: self.authority.clone(),
            path: self.path.clone(),
            status_code: self.status_code.clone(),
            reason: self.reason.clone(),
            headers_text: self.headers_text.clone(),
            body: Arc::clone(&self.body),
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

pub(in crate::llm_pipeline) fn response_candidate_starts_at(bytes: &[u8]) -> bool {
    bytes.starts_with(b"HTTP/")
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
