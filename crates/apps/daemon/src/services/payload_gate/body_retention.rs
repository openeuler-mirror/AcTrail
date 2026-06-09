//! Payload body retention classification.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadSegment, PayloadSourceBoundary,
};
use model_core::process::ProcessIdentity;

const HTTP1_HEADER_SEPARATOR: &[u8] = b"\r\n\r\n";
const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const HTTP2_FRAME_HEADER_BYTES: usize = 9;
const HTTP2_DATA_FRAME_TYPE: u8 = 0x0;
const HTTP2_HEADERS_FRAME_TYPE: u8 = 0x1;
const HTTP2_CONTINUATION_FRAME_TYPE: u8 = 0x9;
const HTTP2_FLAG_PADDED: u8 = 0x8;
const HTTP_REQUEST_METHODS: [&str; 9] = [
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT", "TRACE",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::services) enum PayloadBodyRetention {
    Full,
    SummaryOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::services) struct PayloadBodyRetentionDecision {
    pub(in crate::services) mode: PayloadBodyRetention,
    remember: bool,
    stream_id: Option<u32>,
}

#[derive(Default)]
pub(in crate::services) struct PayloadBodyRetentionGate {
    streams: BTreeMap<BodyStreamKey, PayloadBodyRetention>,
}

impl PayloadBodyRetentionGate {
    pub(in crate::services) fn new() -> Self {
        Self::default()
    }

    pub(in crate::services) fn decide(
        &self,
        segment: &PayloadSegment,
    ) -> PayloadBodyRetentionDecision {
        if !plaintext_http_transport(segment) {
            return PayloadBodyRetentionDecision::transient(PayloadBodyRetention::Full);
        }
        match segment.direction {
            PayloadDirection::Outbound => self.decide_outbound(segment),
            PayloadDirection::Inbound => self.decide_inbound(segment),
        }
    }

    pub(in crate::services) fn apply(
        &mut self,
        segment: &PayloadSegment,
        decision: PayloadBodyRetentionDecision,
    ) {
        if decision.remember {
            self.remember(segment, decision.stream_id, decision.mode);
        }
    }

    pub(in crate::services) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.trace_id != trace_id);
    }

    fn decide_outbound(&self, segment: &PayloadSegment) -> PayloadBodyRetentionDecision {
        if let Some(request) = classify_request(&segment.bytes) {
            let mode = if request.llm {
                PayloadBodyRetention::Full
            } else {
                PayloadBodyRetention::SummaryOnly
            };
            return PayloadBodyRetentionDecision::remember(mode, request.stream_id);
        }
        PayloadBodyRetentionDecision::transient(
            self.lookup(segment, None)
                .unwrap_or(PayloadBodyRetention::Full),
        )
    }

    fn decide_inbound(&self, segment: &PayloadSegment) -> PayloadBodyRetentionDecision {
        if let Some(response) = classify_response(&segment.bytes) {
            if response.llm {
                return PayloadBodyRetentionDecision::remember(
                    PayloadBodyRetention::Full,
                    response.stream_id,
                );
            }
            return PayloadBodyRetentionDecision::transient(
                self.lookup(segment, response.stream_id)
                    .unwrap_or(PayloadBodyRetention::SummaryOnly),
            );
        }
        PayloadBodyRetentionDecision::transient(
            self.lookup(segment, None)
                .unwrap_or(PayloadBodyRetention::Full),
        )
    }

    fn remember(
        &mut self,
        segment: &PayloadSegment,
        stream_id: Option<u32>,
        mode: PayloadBodyRetention,
    ) {
        self.streams
            .insert(BodyStreamKey::new(segment, stream_id), mode);
        if stream_id.is_some() {
            self.streams.insert(BodyStreamKey::new(segment, None), mode);
        }
    }

    fn lookup(
        &self,
        segment: &PayloadSegment,
        stream_id: Option<u32>,
    ) -> Option<PayloadBodyRetention> {
        self.streams
            .get(&BodyStreamKey::new(segment, stream_id))
            .copied()
            .or_else(|| {
                self.streams
                    .get(&BodyStreamKey::new(segment, None))
                    .copied()
            })
    }
}

impl PayloadBodyRetentionDecision {
    fn remember(mode: PayloadBodyRetention, stream_id: Option<u32>) -> Self {
        Self {
            mode,
            remember: true,
            stream_id,
        }
    }

    fn transient(mode: PayloadBodyRetention) -> Self {
        Self {
            mode,
            remember: false,
            stream_id: None,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct BodyStreamKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    source_boundary: &'static str,
    stream_key: String,
    stream_id: Option<u32>,
}

impl BodyStreamKey {
    fn new(segment: &PayloadSegment, stream_id: Option<u32>) -> Self {
        Self {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            source_boundary: source_boundary_name(segment.source_boundary),
            stream_key: segment.stream_key.to_string(),
            stream_id,
        }
    }
}

struct ClassifiedMessage {
    stream_id: Option<u32>,
    llm: bool,
}

fn plaintext_http_transport(segment: &PayloadSegment) -> bool {
    segment.content_state == PayloadContentState::Plaintext
        && matches!(
            segment.source_boundary,
            PayloadSourceBoundary::TlsUserSpace | PayloadSourceBoundary::Syscall
        )
}

fn classify_request(bytes: &[u8]) -> Option<ClassifiedMessage> {
    classify_http1_request(bytes).or_else(|| classify_http2_request(bytes))
}

fn classify_response(bytes: &[u8]) -> Option<ClassifiedMessage> {
    classify_http1_response(bytes).or_else(|| classify_http2_response(bytes))
}

fn classify_http1_request(bytes: &[u8]) -> Option<ClassifiedMessage> {
    let header_end = find_bytes(bytes, HTTP1_HEADER_SEPARATOR)?;
    let header_text = std::str::from_utf8(&bytes[..header_end]).ok()?;
    let first_line = header_text.lines().next()?.trim();
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    parts.next()?;
    let version = parts.next()?;
    if !HTTP_REQUEST_METHODS.contains(&method) || !version.starts_with("HTTP/") {
        return None;
    }
    if method == "CONNECT" {
        return Some(ClassifiedMessage {
            stream_id: None,
            llm: false,
        });
    }
    let body_start = header_end + HTTP1_HEADER_SEPARATOR.len();
    let body = http1_body(bytes, header_text, body_start)?;
    let llm = body_looks_like_llm_request(body.bytes);
    if !llm && !body.complete && body.bytes.is_empty() {
        return None;
    }
    Some(ClassifiedMessage {
        stream_id: None,
        llm,
    })
}

fn classify_http1_response(bytes: &[u8]) -> Option<ClassifiedMessage> {
    let header_end = find_bytes(bytes, HTTP1_HEADER_SEPARATOR)?;
    let header_text = std::str::from_utf8(&bytes[..header_end]).ok()?;
    let first_line = header_text.lines().next()?.trim();
    if !first_line.starts_with("HTTP/") {
        return None;
    }
    let body_start = header_end + HTTP1_HEADER_SEPARATOR.len();
    let body = http1_body(bytes, header_text, body_start)?;
    Some(ClassifiedMessage {
        stream_id: None,
        llm: body_looks_like_llm_response(body.bytes),
    })
}

struct Http1Body<'a> {
    bytes: &'a [u8],
    complete: bool,
}

fn http1_body<'a>(bytes: &'a [u8], header_text: &str, body_start: usize) -> Option<Http1Body<'a>> {
    if let Some(length) = http1_content_length(header_text)? {
        let body_end = body_start.checked_add(length)?;
        let available_end = bytes.len().min(body_end);
        return Some(Http1Body {
            bytes: bytes.get(body_start..available_end).unwrap_or_default(),
            complete: bytes.len() >= body_end,
        });
    }
    Some(Http1Body {
        bytes: bytes.get(body_start..).unwrap_or_default(),
        complete: true,
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

fn classify_http2_request(bytes: &[u8]) -> Option<ClassifiedMessage> {
    let frames = http2_frames(bytes)?;
    if frames.body.is_empty() {
        return None;
    }
    Some(ClassifiedMessage {
        stream_id: frames.stream_id,
        llm: body_looks_like_llm_request(&frames.body),
    })
}

fn classify_http2_response(bytes: &[u8]) -> Option<ClassifiedMessage> {
    let frames = http2_frames(bytes)?;
    if frames.body.is_empty() && !frames.saw_http_frame {
        return None;
    }
    Some(ClassifiedMessage {
        stream_id: frames.stream_id,
        llm: body_looks_like_llm_response(&frames.body),
    })
}

struct Http2FrameBodies {
    stream_id: Option<u32>,
    body: Vec<u8>,
    saw_http_frame: bool,
}

fn http2_frames(bytes: &[u8]) -> Option<Http2FrameBodies> {
    let mut cursor = if bytes.starts_with(HTTP2_CONNECTION_PREFACE) {
        HTTP2_CONNECTION_PREFACE.len()
    } else {
        0
    };
    let mut body = Vec::new();
    let mut stream_id = None;
    let mut saw_http_frame = false;
    while cursor + HTTP2_FRAME_HEADER_BYTES <= bytes.len() {
        let Some(frame) = decode_http2_frame(&bytes[cursor..]) else {
            return None;
        };
        if frame.stream_id != 0 && stream_id.is_none() {
            stream_id = Some(frame.stream_id);
        }
        if matches!(
            frame.frame_type,
            HTTP2_DATA_FRAME_TYPE | HTTP2_HEADERS_FRAME_TYPE | HTTP2_CONTINUATION_FRAME_TYPE
        ) {
            saw_http_frame = true;
        }
        if frame.frame_type == HTTP2_DATA_FRAME_TYPE
            && let Some(data) = http2_data_payload(frame.flags, frame.payload)
        {
            body.extend_from_slice(data);
        }
        cursor += frame.encoded_len;
    }
    (cursor > 0 || bytes.starts_with(HTTP2_CONNECTION_PREFACE)).then_some(Http2FrameBodies {
        stream_id,
        body,
        saw_http_frame,
    })
}

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
    let mut cursor = 0usize;
    let mut data_end = payload.len();
    if flags & HTTP2_FLAG_PADDED != 0 {
        let padding = usize::from(*payload.first()?);
        cursor = cursor.checked_add(1)?;
        data_end = data_end.checked_sub(padding)?;
    }
    if cursor <= data_end && data_end <= payload.len() {
        Some(&payload[cursor..data_end])
    } else {
        None
    }
}

fn body_looks_like_llm_request(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    text.contains("\"model\"")
        && (text.contains("\"messages\"")
            || text.contains("\"prompt\"")
            || text.contains("\"input\""))
}

fn body_looks_like_llm_response(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    text.contains("message_stop")
        || text.contains("[done]")
        || (text.contains("\"model\"")
            && (text.contains("\"choices\"")
                || text.contains("\"content\"")
                || text.contains("\"output\"")))
}

fn source_boundary_name(source_boundary: PayloadSourceBoundary) -> &'static str {
    match source_boundary {
        PayloadSourceBoundary::TlsUserSpace => "tls_user_space",
        PayloadSourceBoundary::Syscall => "syscall",
        PayloadSourceBoundary::Stdio => "stdio",
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
