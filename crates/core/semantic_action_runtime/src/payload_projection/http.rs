//! HTTP message extraction from plaintext transport payloads.

mod stream_id;

use std::collections::BTreeMap;

use crate::payload_projection::encoding::base64_encode;

pub(crate) use stream_id::request_stream_id_hint;

const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const HTTP2_FRAME_HEADER_BYTES: usize = 9;
const HTTP2_DATA_FRAME_TYPE: u8 = 0x0;
const HTTP2_HEADERS_FRAME_TYPE: u8 = 0x1;
const HTTP2_CONTINUATION_FRAME_TYPE: u8 = 0x9;
const HTTP2_FLAG_PADDED: u8 = 0x8;
const HTTP2_FLAG_PRIORITY: u8 = 0x20;
const HTTP1_HEADER_SEPARATOR: &[u8] = b"\r\n\r\n";
const HTTP1_LINE_ENDING: &[u8] = b"\r\n";
const HTTP1_RESPONSE_PREFIX: &str = "HTTP/";
const HTTP1_REQUEST_METHODS: [&str; 9] = [
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT", "TRACE",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpRequestParts {
    pub(crate) protocol: &'static str,
    pub(crate) scheme: &'static str,
    pub(crate) method: Option<String>,
    pub(crate) authority: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) stream_id: Option<u32>,
    pub(crate) headers_text: Option<String>,
    pub(crate) headers_hpack_base64: Option<String>,
    pub(crate) body: Vec<u8>,
    pub(crate) encoded_len: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpResponseParts {
    pub(crate) protocol: &'static str,
    pub(crate) scheme: &'static str,
    pub(crate) status_code: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) stream_id: Option<u32>,
    pub(crate) headers_text: Option<String>,
    pub(crate) headers_hpack_base64: Option<String>,
    pub(crate) body: Vec<u8>,
    pub(crate) encoded_len: usize,
    pub(crate) complete: bool,
    pub(crate) body_boundary_known: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpSplitBatch<T> {
    pub(crate) messages: Vec<HttpMessageRange<T>>,
    pub(crate) consumed_len: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpMessageRange<T> {
    pub(crate) parts: T,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

pub(crate) fn split_request(bytes: &[u8]) -> Option<HttpRequestParts> {
    split_http1_request(bytes).or_else(|| split_http2_request(bytes))
}

pub(crate) fn split_request_batch(bytes: &[u8]) -> Option<HttpSplitBatch<HttpRequestParts>> {
    if let Some(parts) = split_http1_request(bytes) {
        let end = parts.encoded_len;
        return Some(HttpSplitBatch {
            messages: vec![HttpMessageRange {
                parts,
                start: 0,
                end,
            }],
            consumed_len: end,
        });
    }
    split_http2_request_batch(bytes)
}

pub(crate) fn request_prefix_skip_len(bytes: &[u8]) -> Option<usize> {
    if http1_request_starts_at(bytes) {
        return None;
    }
    first_http1_request_start_after_prefix(bytes)
}

pub(crate) fn split_response(bytes: &[u8]) -> Option<HttpResponseParts> {
    split_http1_response(bytes).or_else(|| split_http2_response(bytes))
}

pub(crate) fn split_response_batch(bytes: &[u8]) -> Option<HttpSplitBatch<HttpResponseParts>> {
    if let Some(parts) = split_http1_response(bytes) {
        let end = parts.encoded_len;
        return Some(HttpSplitBatch {
            messages: vec![HttpMessageRange {
                parts,
                start: 0,
                end,
            }],
            consumed_len: end,
        });
    }
    split_http2_response_batch(bytes)
}

pub(crate) fn split_http1_request(bytes: &[u8]) -> Option<HttpRequestParts> {
    let separator = find_bytes(bytes, HTTP1_HEADER_SEPARATOR)?;
    let header_bytes = &bytes[..separator];
    let header_text = String::from_utf8_lossy(header_bytes).into_owned();
    let request_line = header_text.split("\r\n").next()?;
    let (method, path) = parse_http1_request_line(request_line)?;
    let authority = header_text.split("\r\n").find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case("host")
            .then(|| value.trim().to_string())
    });
    let body_start = separator + HTTP1_HEADER_SEPARATOR.len();
    let transfer_is_chunked = http1_header_value(&header_text, "transfer-encoding")
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    let (body, body_end) = if transfer_is_chunked {
        let chunked = parse_http1_chunked_body_prefix(&bytes[body_start..])?;
        if !chunked.complete {
            return None;
        }
        (chunked.body, body_start.checked_add(chunked.consumed_len)?)
    } else {
        let body_end = match http1_content_length(&header_text)? {
            Some(length) => body_start.checked_add(length)?,
            None => body_start,
        };
        if bytes.len() < body_end {
            return None;
        }
        (bytes[body_start..body_end].to_vec(), body_end)
    };
    Some(HttpRequestParts {
        protocol: "http/1.1",
        scheme: "https",
        method: Some(method),
        authority,
        path,
        stream_id: None,
        headers_text: Some(header_text.to_string()),
        headers_hpack_base64: None,
        body,
        encoded_len: body_end,
    })
}

fn split_http1_response(bytes: &[u8]) -> Option<HttpResponseParts> {
    let separator = find_bytes(bytes, HTTP1_HEADER_SEPARATOR)?;
    let header_bytes = &bytes[..separator];
    let header_text = String::from_utf8_lossy(header_bytes).into_owned();
    let (protocol, status_code, reason) = parse_http1_status_line(&header_text)?;
    let body_start = separator + HTTP1_HEADER_SEPARATOR.len();
    let body_bytes = &bytes[body_start..];
    let content_length = match http1_header_value(&header_text, "content-length") {
        Some(value) => Some(value.parse::<usize>().ok()?),
        None => None,
    };
    let transfer_is_chunked = http1_header_value(&header_text, "transfer-encoding")
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    let mut body_boundary_known = content_length.is_some();
    let (body, body_len, complete) = if let Some(length) = content_length {
        let available = length.min(body_bytes.len());
        (
            body_bytes[..available].to_vec(),
            available,
            available == length,
        )
    } else if transfer_is_chunked {
        let chunked = parse_http1_chunked_body_prefix(body_bytes)?;
        body_boundary_known = chunked.boundary_known;
        (chunked.body, chunked.consumed_len, chunked.complete)
    } else {
        (body_bytes.to_vec(), body_bytes.len(), false)
    };
    let encoded_len = body_start.checked_add(body_len)?;
    Some(HttpResponseParts {
        protocol,
        scheme: "https",
        status_code: Some(status_code),
        reason,
        stream_id: None,
        headers_text: Some(header_text),
        headers_hpack_base64: None,
        body,
        encoded_len,
        complete,
        body_boundary_known,
    })
}

fn split_http2_request(bytes: &[u8]) -> Option<HttpRequestParts> {
    let mut cursor = if bytes.starts_with(HTTP2_CONNECTION_PREFACE) {
        HTTP2_CONNECTION_PREFACE.len()
    } else {
        0
    };
    let mut body = Vec::new();
    let mut header_block = Vec::new();
    let mut stream_id = None;

    while cursor + HTTP2_FRAME_HEADER_BYTES <= bytes.len() {
        let Some(frame) = decode_http2_frame(&bytes[cursor..]) else {
            cursor += 1;
            continue;
        };
        if frame.stream_id != 0 && stream_id.is_none() {
            stream_id = Some(frame.stream_id);
        }
        match frame.frame_type {
            HTTP2_DATA_FRAME_TYPE => {
                if let Some(data) = http2_data_payload(frame.flags, frame.payload) {
                    body.extend_from_slice(data);
                }
            }
            HTTP2_HEADERS_FRAME_TYPE => {
                if let Some(headers) = http2_headers_payload(frame.flags, frame.payload) {
                    header_block.extend_from_slice(headers);
                }
            }
            HTTP2_CONTINUATION_FRAME_TYPE => {
                header_block.extend_from_slice(frame.payload);
            }
            _ => {}
        }
        cursor += frame.encoded_len;
    }

    if body.is_empty() || header_block.is_empty() {
        return None;
    }
    Some(HttpRequestParts {
        protocol: "h2",
        scheme: "https",
        method: None,
        authority: None,
        path: None,
        stream_id,
        headers_text: None,
        headers_hpack_base64: (!header_block.is_empty()).then(|| base64_encode(&header_block)),
        body,
        encoded_len: bytes.len(),
    })
}

fn split_http2_response(bytes: &[u8]) -> Option<HttpResponseParts> {
    let mut cursor = if bytes.starts_with(HTTP2_CONNECTION_PREFACE) {
        HTTP2_CONNECTION_PREFACE.len()
    } else {
        0
    };
    let mut body = Vec::new();
    let mut header_block = Vec::new();
    let mut stream_id = None;

    while cursor + HTTP2_FRAME_HEADER_BYTES <= bytes.len() {
        let Some(frame) = decode_http2_frame(&bytes[cursor..]) else {
            cursor += 1;
            continue;
        };
        if frame.stream_id != 0 && stream_id.is_none() {
            stream_id = Some(frame.stream_id);
        }
        match frame.frame_type {
            HTTP2_DATA_FRAME_TYPE => {
                if let Some(data) = http2_data_payload(frame.flags, frame.payload) {
                    body.extend_from_slice(data);
                }
            }
            HTTP2_HEADERS_FRAME_TYPE => {
                if let Some(headers) = http2_headers_payload(frame.flags, frame.payload) {
                    header_block.extend_from_slice(headers);
                }
            }
            HTTP2_CONTINUATION_FRAME_TYPE => {
                header_block.extend_from_slice(frame.payload);
            }
            _ => {}
        }
        cursor += frame.encoded_len;
    }

    if body.is_empty() {
        return None;
    }
    Some(HttpResponseParts {
        protocol: "h2",
        scheme: "https",
        status_code: None,
        reason: None,
        stream_id,
        headers_text: None,
        headers_hpack_base64: (!header_block.is_empty()).then(|| base64_encode(&header_block)),
        body,
        encoded_len: bytes.len(),
        complete: false,
        body_boundary_known: false,
    })
}

fn split_http2_request_batch(bytes: &[u8]) -> Option<HttpSplitBatch<HttpRequestParts>> {
    let accumulated = accumulate_http2_message_frames(bytes)?;
    let consumed_len = http2_batch_consumed_len(
        &accumulated.streams,
        accumulated.decoded_end,
        http2_request_accumulator_is_complete,
    );
    let messages = accumulated
        .streams
        .into_iter()
        .filter_map(|(stream_id, accumulated)| {
            if !http2_request_accumulator_is_complete(&accumulated)
                || accumulated.end > consumed_len
            {
                return None;
            }
            let start = accumulated.start?;
            let end = accumulated.end;
            Some(HttpMessageRange {
                parts: HttpRequestParts {
                    protocol: "h2",
                    scheme: "https",
                    method: None,
                    authority: None,
                    path: None,
                    stream_id: Some(stream_id),
                    headers_text: None,
                    headers_hpack_base64: (!accumulated.header_block.is_empty())
                        .then(|| base64_encode(&accumulated.header_block)),
                    body: accumulated.body,
                    encoded_len: end.saturating_sub(start),
                },
                start,
                end,
            })
        })
        .collect::<Vec<_>>();
    if messages.is_empty() && consumed_len == 0 {
        return None;
    }
    Some(HttpSplitBatch {
        messages,
        consumed_len,
    })
}

fn split_http2_response_batch(bytes: &[u8]) -> Option<HttpSplitBatch<HttpResponseParts>> {
    let accumulated = accumulate_http2_message_frames(bytes)?;
    let consumed_len = http2_batch_consumed_len(
        &accumulated.streams,
        accumulated.decoded_end,
        http2_response_accumulator_is_complete,
    );
    let messages = accumulated
        .streams
        .into_iter()
        .filter_map(|(stream_id, accumulated)| {
            if !http2_response_accumulator_is_complete(&accumulated)
                || accumulated.end > consumed_len
            {
                return None;
            }
            let start = accumulated.start?;
            let end = accumulated.end;
            Some(HttpMessageRange {
                parts: HttpResponseParts {
                    protocol: "h2",
                    scheme: "https",
                    status_code: None,
                    reason: None,
                    stream_id: Some(stream_id),
                    headers_text: None,
                    headers_hpack_base64: (!accumulated.header_block.is_empty())
                        .then(|| base64_encode(&accumulated.header_block)),
                    body: accumulated.body,
                    encoded_len: end.saturating_sub(start),
                    complete: false,
                    body_boundary_known: false,
                },
                start,
                end,
            })
        })
        .collect::<Vec<_>>();
    if messages.is_empty() && consumed_len == 0 {
        return None;
    }
    Some(HttpSplitBatch {
        messages,
        consumed_len,
    })
}

struct Http2AccumulatedFrames {
    streams: BTreeMap<u32, Http2MessageAccumulator>,
    decoded_end: usize,
}

#[derive(Default)]
struct Http2MessageAccumulator {
    body: Vec<u8>,
    header_block: Vec<u8>,
    start: Option<usize>,
    end: usize,
}

impl Http2MessageAccumulator {
    fn push_body(&mut self, start: usize, end: usize, data: &[u8]) {
        self.push_part(start, end);
        self.body.extend_from_slice(data);
    }

    fn push_header(&mut self, start: usize, end: usize, data: &[u8]) {
        self.push_part(start, end);
        self.header_block.extend_from_slice(data);
    }

    fn push_part(&mut self, start: usize, end: usize) {
        if self.start.is_none() {
            self.start = Some(start);
        }
        self.end = self.end.max(end);
    }
}

fn accumulate_http2_message_frames(bytes: &[u8]) -> Option<Http2AccumulatedFrames> {
    let mut cursor = if bytes.starts_with(HTTP2_CONNECTION_PREFACE) {
        HTTP2_CONNECTION_PREFACE.len()
    } else {
        0
    };
    let first_decode = decode_http2_frame_at(&bytes[cursor..]);
    if cursor == 0 && !matches!(first_decode, Http2FrameDecode::Complete(_)) {
        return None;
    }

    let mut streams = BTreeMap::<u32, Http2MessageAccumulator>::new();
    let mut decoded_end = cursor;
    loop {
        match decode_http2_frame_at(&bytes[cursor..]) {
            Http2FrameDecode::Complete(frame) => {
                let start = cursor;
                let end = cursor + frame.encoded_len;
                accumulate_http2_frame(&mut streams, start, end, frame);
                cursor = end;
                decoded_end = end;
            }
            Http2FrameDecode::Incomplete => break,
            Http2FrameDecode::Invalid => {
                cursor += 1;
                decoded_end = cursor;
            }
        }
    }

    Some(Http2AccumulatedFrames {
        streams,
        decoded_end,
    })
}

fn accumulate_http2_frame(
    streams: &mut BTreeMap<u32, Http2MessageAccumulator>,
    start: usize,
    end: usize,
    frame: Http2Frame<'_>,
) {
    if frame.stream_id == 0 {
        return;
    }
    match frame.frame_type {
        HTTP2_DATA_FRAME_TYPE => {
            if let Some(data) = http2_data_payload(frame.flags, frame.payload) {
                streams
                    .entry(frame.stream_id)
                    .or_default()
                    .push_body(start, end, data);
            }
        }
        HTTP2_HEADERS_FRAME_TYPE => {
            if let Some(headers) = http2_headers_payload(frame.flags, frame.payload) {
                streams
                    .entry(frame.stream_id)
                    .or_default()
                    .push_header(start, end, headers);
            }
        }
        HTTP2_CONTINUATION_FRAME_TYPE => {
            streams
                .entry(frame.stream_id)
                .or_default()
                .push_header(start, end, frame.payload);
        }
        _ => {}
    }
}

fn http2_batch_consumed_len(
    streams: &BTreeMap<u32, Http2MessageAccumulator>,
    decoded_end: usize,
    is_complete: fn(&Http2MessageAccumulator) -> bool,
) -> usize {
    let Some(earliest_incomplete_start) = streams
        .values()
        .filter(|accumulated| !is_complete(accumulated))
        .filter_map(|accumulated| accumulated.start)
        .min()
    else {
        return decoded_end;
    };

    streams
        .values()
        .filter(|accumulated| is_complete(accumulated))
        .filter_map(|accumulated| accumulated.start.map(|start| (start, accumulated.end)))
        .filter(|(start, end)| {
            *start < earliest_incomplete_start && *end > earliest_incomplete_start
        })
        .map(|(start, _)| start)
        .min()
        .unwrap_or(earliest_incomplete_start)
}

fn http2_request_accumulator_is_complete(accumulated: &Http2MessageAccumulator) -> bool {
    !accumulated.body.is_empty() && !accumulated.header_block.is_empty()
}

fn http2_response_accumulator_is_complete(accumulated: &Http2MessageAccumulator) -> bool {
    !accumulated.body.is_empty()
}

fn http1_header_value<'a>(header_text: &'a str, name: &str) -> Option<&'a str> {
    header_text.split("\r\n").find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn http1_content_length(header_text: &str) -> Option<Option<usize>> {
    for line in header_text.split("\r\n") {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().ok().map(Some);
        }
    }
    Some(None)
}

fn parse_http1_status_line(header_text: &str) -> Option<(&'static str, String, Option<String>)> {
    let first_line = header_text.lines().next()?.trim();
    if !first_line.starts_with(HTTP1_RESPONSE_PREFIX) {
        return None;
    }
    let mut parts = first_line.splitn(3, ' ');
    let protocol = http1_response_protocol(parts.next()?)?;
    let status_code = parts.next()?.to_string();
    let reason = parts.next().map(ToString::to_string);
    Some((protocol, status_code, reason))
}

fn http1_response_protocol(version: &str) -> Option<&'static str> {
    match version {
        "HTTP/1.0" => Some("http/1.0"),
        "HTTP/1.1" => Some("http/1.1"),
        _ => None,
    }
}

struct Http1ChunkedBodyPrefix {
    body: Vec<u8>,
    consumed_len: usize,
    complete: bool,
    boundary_known: bool,
}

fn parse_http1_chunked_body_prefix(bytes: &[u8]) -> Option<Http1ChunkedBodyPrefix> {
    if starts_like_sse_body(bytes) {
        return Some(Http1ChunkedBodyPrefix {
            body: bytes.to_vec(),
            consumed_len: bytes.len(),
            complete: false,
            boundary_known: false,
        });
    }

    let mut cursor = 0;
    let mut body = Vec::new();
    let mut parsed_chunk = false;
    loop {
        let Some(line_end) = find_bytes(&bytes[cursor..], HTTP1_LINE_ENDING) else {
            return parsed_chunk.then_some(Http1ChunkedBodyPrefix {
                body,
                consumed_len: bytes.len(),
                complete: false,
                boundary_known: true,
            });
        };
        let size_line = &bytes[cursor..cursor + line_end];
        let size_text = std::str::from_utf8(size_line)
            .ok()?
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        let size = usize::from_str_radix(size_text, 16).ok()?;
        cursor = cursor
            .checked_add(line_end)?
            .checked_add(HTTP1_LINE_ENDING.len())?;
        let data_end = cursor.checked_add(size)?;
        if bytes.len() < data_end {
            return Some(Http1ChunkedBodyPrefix {
                body,
                consumed_len: bytes.len(),
                complete: false,
                boundary_known: true,
            });
        }
        body.extend_from_slice(&bytes[cursor..data_end]);
        let chunk_end = data_end.checked_add(HTTP1_LINE_ENDING.len())?;
        if bytes.len() < chunk_end {
            return Some(Http1ChunkedBodyPrefix {
                body,
                consumed_len: bytes.len(),
                complete: false,
                boundary_known: true,
            });
        }
        cursor = chunk_end;
        parsed_chunk = true;
        if size == 0 {
            return Some(Http1ChunkedBodyPrefix {
                body,
                consumed_len: cursor,
                complete: true,
                boundary_known: true,
            });
        }
    }
}

fn starts_like_sse_body(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .next()
        .map(str::trim)
        .is_some_and(|line| line.starts_with("event:") || line.starts_with("data:"))
}

fn parse_http1_request_line(line: &str) -> Option<(String, Option<String>)> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    if !HTTP1_REQUEST_METHODS.contains(&method) {
        return None;
    }
    let path = parts.next().map(ToString::to_string);
    parts
        .next()?
        .starts_with("HTTP/")
        .then(|| (method.to_string(), path))
}

fn http1_request_starts_at(bytes: &[u8]) -> bool {
    let Some(line_end) = find_bytes(bytes, HTTP1_LINE_ENDING) else {
        return false;
    };
    std::str::from_utf8(&bytes[..line_end])
        .ok()
        .and_then(parse_http1_request_line)
        .is_some()
}

fn first_http1_request_start_after_prefix(bytes: &[u8]) -> Option<usize> {
    (1..bytes.len()).find(|offset| {
        http1_request_method_starts_at(&bytes[*offset..])
            && http1_request_starts_at(&bytes[*offset..])
    })
}

fn http1_request_method_starts_at(bytes: &[u8]) -> bool {
    HTTP1_REQUEST_METHODS.iter().any(|method| {
        let method = method.as_bytes();
        bytes.starts_with(method) && bytes.get(method.len()) == Some(&b' ')
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Http2Frame<'a> {
    frame_type: u8,
    flags: u8,
    stream_id: u32,
    payload: &'a [u8],
    encoded_len: usize,
}

fn decode_http2_frame(bytes: &[u8]) -> Option<Http2Frame<'_>> {
    match decode_http2_frame_at(bytes) {
        Http2FrameDecode::Complete(frame) => Some(frame),
        Http2FrameDecode::Incomplete | Http2FrameDecode::Invalid => None,
    }
}

enum Http2FrameDecode<'a> {
    Complete(Http2Frame<'a>),
    Incomplete,
    Invalid,
}

fn decode_http2_frame_at(bytes: &[u8]) -> Http2FrameDecode<'_> {
    if bytes.len() < HTTP2_FRAME_HEADER_BYTES {
        return Http2FrameDecode::Incomplete;
    }
    let length =
        (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
    let Some(encoded_len) = HTTP2_FRAME_HEADER_BYTES.checked_add(length) else {
        return Http2FrameDecode::Invalid;
    };
    if bytes.len() < encoded_len {
        return Http2FrameDecode::Incomplete;
    }
    let frame_type = bytes[3];
    let stream_id = (u32::from(bytes[5] & 0x7f) << 24)
        | (u32::from(bytes[6]) << 16)
        | (u32::from(bytes[7]) << 8)
        | u32::from(bytes[8]);
    if !http2_stream_id_is_valid(frame_type, stream_id) {
        return Http2FrameDecode::Invalid;
    }
    Http2FrameDecode::Complete(Http2Frame {
        frame_type,
        flags: bytes[4],
        stream_id,
        payload: &bytes[HTTP2_FRAME_HEADER_BYTES..encoded_len],
        encoded_len,
    })
}

fn http2_stream_id_is_valid(frame_type: u8, stream_id: u32) -> bool {
    match frame_type {
        0x0 | 0x1 | 0x2 | 0x3 | 0x5 | 0x9 => stream_id != 0,
        0x4 | 0x6 | 0x7 => stream_id == 0,
        0x8 => true,
        _ => false,
    }
}

fn http2_data_payload(flags: u8, payload: &[u8]) -> Option<&[u8]> {
    strip_http2_padding(flags, payload, 0)
}

fn http2_headers_payload(flags: u8, payload: &[u8]) -> Option<&[u8]> {
    let priority_bytes = if flags & HTTP2_FLAG_PRIORITY == 0 {
        0
    } else {
        5
    };
    strip_http2_padding(flags, payload, priority_bytes)
}

fn strip_http2_padding(flags: u8, payload: &[u8], prefix_without_padding: usize) -> Option<&[u8]> {
    let mut start = 0;
    let mut end = payload.len();
    if flags & HTTP2_FLAG_PADDED != 0 {
        let padding = usize::from(*payload.first()?);
        start = 1;
        if padding > end.saturating_sub(start) {
            return None;
        }
        end -= padding;
    }
    start = start.checked_add(prefix_without_padding)?;
    if start > end {
        return None;
    }
    Some(&payload[start..end])
}
