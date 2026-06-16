//! Payload body retention classification.

use std::collections::BTreeMap;

use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadSegment, PayloadSourceBoundary,
};
use model_core::process::ProcessIdentity;

#[path = "body_retention_http2.rs"]
mod http2;

const HTTP1_HEADER_SEPARATOR: &[u8] = b"\r\n\r\n";
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
    pub(in crate::services) semantic_layer: PayloadSemanticLayer,
    remember: bool,
    stream_id: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::services) enum PayloadSemanticLayer {
    None,
    Http,
    Llm,
}

impl PayloadSemanticLayer {
    pub(in crate::services) fn consumed_by_higher_layer(self) -> bool {
        matches!(self, Self::Http | Self::Llm)
    }

    pub(in crate::services) fn consumed_by_llm(self) -> bool {
        matches!(self, Self::Llm)
    }
}

pub(in crate::services) struct PayloadBodyRetentionGate {
    streams: BTreeMap<BodyStreamKey, PayloadBodyRetentionMemo>,
    http2_probe_bytes: BTreeMap<BodyStreamKey, u64>,
    http2_probe_max_bytes: u64,
    semantic_retention: SemanticRetentionConfig,
}

impl PayloadBodyRetentionGate {
    pub(in crate::services) fn new(
        http2_probe_max_bytes: u64,
        semantic_retention: SemanticRetentionConfig,
    ) -> Self {
        Self {
            streams: BTreeMap::new(),
            http2_probe_bytes: BTreeMap::new(),
            http2_probe_max_bytes,
            semantic_retention,
        }
    }

    pub(in crate::services) fn decide(
        &mut self,
        segment: &PayloadSegment,
    ) -> PayloadBodyRetentionDecision {
        if !plaintext_http_transport(segment) {
            return PayloadBodyRetentionDecision::transient(
                PayloadBodyRetention::Full,
                PayloadSemanticLayer::None,
            );
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
            self.remember(
                segment,
                decision.stream_id,
                decision.mode,
                decision.semantic_layer,
            );
        }
    }

    pub(in crate::services) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.trace_id != trace_id);
        self.http2_probe_bytes
            .retain(|key, _| key.trace_id != trace_id);
    }

    fn decide_outbound(&mut self, segment: &PayloadSegment) -> PayloadBodyRetentionDecision {
        if let Some(request) = classify_http1_request(&segment.bytes) {
            if request.llm {
                return PayloadBodyRetentionDecision::remember(
                    self.http_analysis_mode(true),
                    PayloadSemanticLayer::Llm,
                    request.stream_id,
                );
            }
            if request.summary_only_safe {
                return PayloadBodyRetentionDecision::remember(
                    self.http_analysis_mode(false),
                    PayloadSemanticLayer::Http,
                    request.stream_id,
                );
            }
        }
        if let Some(request) = http2::classify_request(&segment.bytes) {
            if request.llm {
                return PayloadBodyRetentionDecision::remember(
                    self.http_analysis_mode(true),
                    PayloadSemanticLayer::Llm,
                    request.stream_id,
                );
            }
            return self.decide_http2_probe(segment, request.stream_id);
        }
        if let Some(stream_id) = http2::candidate_stream_id(&segment.bytes) {
            return self.decide_http2_probe(segment, stream_id);
        }
        if http2::body_looks_like_llm_request(&segment.bytes) {
            return PayloadBodyRetentionDecision::remember(
                self.http_analysis_mode(true),
                PayloadSemanticLayer::Llm,
                None,
            );
        }
        PayloadBodyRetentionDecision::transient_memo(
            self.lookup_with_fallback(segment, None)
                .unwrap_or(PayloadBodyRetentionMemo {
                    mode: PayloadBodyRetention::Full,
                    semantic_layer: PayloadSemanticLayer::None,
                }),
        )
    }

    fn decide_inbound(&self, segment: &PayloadSegment) -> PayloadBodyRetentionDecision {
        if let Some(response) = classify_http1_response(&segment.bytes) {
            if response.llm {
                return PayloadBodyRetentionDecision::remember(
                    self.http_analysis_mode(true),
                    PayloadSemanticLayer::Llm,
                    response.stream_id,
                );
            }
            return PayloadBodyRetentionDecision::transient_memo(
                self.lookup_with_fallback(segment, response.stream_id)
                    .unwrap_or(PayloadBodyRetentionMemo {
                        mode: self.http_analysis_mode(false),
                        semantic_layer: PayloadSemanticLayer::Http,
                    }),
            );
        }
        if let Some(response) = http2::classify_response(&segment.bytes) {
            if response.llm {
                return PayloadBodyRetentionDecision::remember(
                    self.http_analysis_mode(true),
                    PayloadSemanticLayer::Llm,
                    response.stream_id,
                );
            }
            return PayloadBodyRetentionDecision::transient_memo(
                self.lookup_exact(segment, response.stream_id).unwrap_or(
                    PayloadBodyRetentionMemo {
                        mode: self.http_analysis_mode(false),
                        semantic_layer: PayloadSemanticLayer::Http,
                    },
                ),
            );
        }
        if let Some(stream_id) = http2::candidate_stream_id(&segment.bytes) {
            return PayloadBodyRetentionDecision::transient_memo(
                self.lookup_exact(segment, stream_id)
                    .unwrap_or(PayloadBodyRetentionMemo {
                        mode: PayloadBodyRetention::SummaryOnly,
                        semantic_layer: PayloadSemanticLayer::None,
                    }),
            );
        }
        if http2::body_looks_like_llm_response(&segment.bytes) {
            return PayloadBodyRetentionDecision::remember(
                self.http_analysis_mode(true),
                PayloadSemanticLayer::Llm,
                None,
            );
        }
        PayloadBodyRetentionDecision::transient_memo(
            self.lookup_with_fallback(segment, None)
                .unwrap_or(PayloadBodyRetentionMemo {
                    mode: PayloadBodyRetention::Full,
                    semantic_layer: PayloadSemanticLayer::None,
                }),
        )
    }

    fn decide_http2_probe(
        &mut self,
        segment: &PayloadSegment,
        stream_id: Option<u32>,
    ) -> PayloadBodyRetentionDecision {
        if let Some(mode) = self.lookup_exact(segment, stream_id) {
            return PayloadBodyRetentionDecision::transient_memo(mode);
        }
        let key = BodyStreamKey::new(segment, stream_id);
        let used = self.http2_probe_bytes.get(&key).copied().unwrap_or(0);
        let Some(next) = used.checked_add(segment.captured_size) else {
            return PayloadBodyRetentionDecision::remember(
                PayloadBodyRetention::SummaryOnly,
                PayloadSemanticLayer::None,
                stream_id,
            );
        };
        if next > self.http2_probe_max_bytes {
            return PayloadBodyRetentionDecision::remember(
                PayloadBodyRetention::SummaryOnly,
                PayloadSemanticLayer::None,
                stream_id,
            );
        }
        self.http2_probe_bytes.insert(key, next);
        PayloadBodyRetentionDecision::transient(
            PayloadBodyRetention::Full,
            PayloadSemanticLayer::None,
        )
    }

    fn remember(
        &mut self,
        segment: &PayloadSegment,
        stream_id: Option<u32>,
        mode: PayloadBodyRetention,
        semantic_layer: PayloadSemanticLayer,
    ) {
        let key = BodyStreamKey::new(segment, stream_id);
        self.http2_probe_bytes.remove(&key);
        self.streams.insert(
            key,
            PayloadBodyRetentionMemo {
                mode,
                semantic_layer,
            },
        );
    }

    fn lookup_exact(
        &self,
        segment: &PayloadSegment,
        stream_id: Option<u32>,
    ) -> Option<PayloadBodyRetentionMemo> {
        self.streams
            .get(&BodyStreamKey::new(segment, stream_id))
            .copied()
    }

    fn lookup_with_fallback(
        &self,
        segment: &PayloadSegment,
        stream_id: Option<u32>,
    ) -> Option<PayloadBodyRetentionMemo> {
        self.lookup_exact(segment, stream_id)
            .or_else(|| self.lookup_exact(segment, None))
    }

    fn http_analysis_mode(&self, llm_message: bool) -> PayloadBodyRetention {
        if self
            .semantic_retention
            .http_body_content_needed(llm_message)
        {
            PayloadBodyRetention::Full
        } else {
            PayloadBodyRetention::SummaryOnly
        }
    }
}

impl PayloadBodyRetentionDecision {
    fn remember(
        mode: PayloadBodyRetention,
        semantic_layer: PayloadSemanticLayer,
        stream_id: Option<u32>,
    ) -> Self {
        Self {
            mode,
            semantic_layer,
            remember: true,
            stream_id,
        }
    }

    fn transient(mode: PayloadBodyRetention, semantic_layer: PayloadSemanticLayer) -> Self {
        Self {
            mode,
            semantic_layer,
            remember: false,
            stream_id: None,
        }
    }

    fn transient_memo(memo: PayloadBodyRetentionMemo) -> Self {
        Self {
            mode: memo.mode,
            semantic_layer: memo.semantic_layer,
            remember: false,
            stream_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PayloadBodyRetentionMemo {
    mode: PayloadBodyRetention,
    semantic_layer: PayloadSemanticLayer,
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
    summary_only_safe: bool,
}

fn plaintext_http_transport(segment: &PayloadSegment) -> bool {
    segment.content_state == PayloadContentState::Plaintext
        && matches!(
            segment.source_boundary,
            PayloadSourceBoundary::TlsUserSpace | PayloadSourceBoundary::Syscall
        )
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
            summary_only_safe: true,
        });
    }
    let body_start = header_end + HTTP1_HEADER_SEPARATOR.len();
    let body = http1_body(bytes, header_text, body_start)?;
    let llm = http2::body_looks_like_llm_request(body.bytes);
    if !llm && !body.complete && body.bytes.is_empty() {
        return None;
    }
    Some(ClassifiedMessage {
        stream_id: None,
        llm,
        summary_only_safe: body.complete,
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
        llm: http2::body_looks_like_llm_response(body.bytes),
        summary_only_safe: false,
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
