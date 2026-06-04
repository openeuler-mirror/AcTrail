//! HTTP request extraction from plaintext transport payloads.

use crate::payload_projection::encoding::base64_encode;

const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const HTTP2_FRAME_HEADER_BYTES: usize = 9;
const HTTP2_DATA_FRAME_TYPE: u8 = 0x0;
const HTTP2_HEADERS_FRAME_TYPE: u8 = 0x1;
const HTTP2_CONTINUATION_FRAME_TYPE: u8 = 0x9;
const HTTP2_FLAG_PADDED: u8 = 0x8;
const HTTP2_FLAG_PRIORITY: u8 = 0x20;
const HTTP1_HEADER_SEPARATOR: &[u8] = b"\r\n\r\n";
const HTTP1_REQUEST_METHODS: [&str; 9] = [
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT", "TRACE",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HttpRequestParts {
    pub protocol: &'static str,
    pub scheme: &'static str,
    pub method: Option<String>,
    pub authority: Option<String>,
    pub path: Option<String>,
    pub stream_id: Option<u32>,
    pub headers_text: Option<String>,
    pub headers_hpack_base64: Option<String>,
    pub body: Vec<u8>,
    pub encoded_len: usize,
}

pub(super) fn split_request(bytes: &[u8]) -> Option<HttpRequestParts> {
    split_http1_request(bytes).or_else(|| split_http2_request(bytes))
}

pub(super) fn split_http1_request(bytes: &[u8]) -> Option<HttpRequestParts> {
    let separator = find_bytes(bytes, HTTP1_HEADER_SEPARATOR)?;
    let header_bytes = &bytes[..separator];
    let header_text = String::from_utf8_lossy(header_bytes).into_owned();
    if !looks_like_http1_header_block(&header_text) {
        return None;
    }
    let request_line = header_text
        .split("\r\n")
        .find(|line| line.contains(" HTTP/"));
    let (method, path) = request_line
        .and_then(parse_http1_request_line)
        .unwrap_or((None, None));
    let authority = header_text.split("\r\n").find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case("host")
            .then(|| value.trim().to_string())
    });
    let body_start = separator + HTTP1_HEADER_SEPARATOR.len();
    let body_end = match http1_content_length(&header_text)? {
        Some(length) => body_start.checked_add(length)?,
        None => body_start,
    };
    if bytes.len() < body_end {
        return None;
    }
    Some(HttpRequestParts {
        protocol: "http/1.1",
        scheme: "https",
        method,
        authority,
        path,
        stream_id: None,
        headers_text: Some(header_text.to_string()),
        headers_hpack_base64: None,
        body: bytes[body_start..body_end].to_vec(),
        encoded_len: body_end,
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

fn looks_like_http1_header_block(text: &str) -> bool {
    text.contains(" HTTP/")
        || text.split("\r\n").any(|line| {
            line.split_once(':')
                .is_some_and(|(key, _)| is_common_http_header(key))
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

fn is_common_http_header(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "host" | "content-length" | "content-type" | "accept" | "user-agent" | "authorization"
    )
}

fn parse_http1_request_line(line: &str) -> Option<(Option<String>, Option<String>)> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    if !HTTP1_REQUEST_METHODS.contains(&method) {
        return Some((None, None));
    }
    Some((
        Some(method.to_string()),
        parts.next().map(ToString::to_string),
    ))
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
    if bytes.len() < HTTP2_FRAME_HEADER_BYTES {
        return None;
    }
    let length =
        (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
    let encoded_len = HTTP2_FRAME_HEADER_BYTES.checked_add(length)?;
    if bytes.len() < encoded_len {
        return None;
    }
    let frame_type = bytes[3];
    let stream_id = (u32::from(bytes[5] & 0x7f) << 24)
        | (u32::from(bytes[6]) << 16)
        | (u32::from(bytes[7]) << 8)
        | u32::from(bytes[8]);
    if !http2_stream_id_is_valid(frame_type, stream_id) {
        return None;
    }
    Some(Http2Frame {
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
