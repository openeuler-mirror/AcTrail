//! HTTP/1.x and SSE parsing helpers for application semantic analysis.

use std::collections::BTreeMap;

use config_core::daemon::{
    ApplicationProtocolConfig, HttpBodyRetention, HttpHeadersRetention, SemanticRetentionConfig,
};
use model_core::event::ApplicationPayload;
use model_core::payload::PayloadSegment;
use serde_json::{Map, Value};

use super::super::base64_encode;

#[path = "parser/streaming.rs"]
mod streaming;

pub(super) fn starts_like_http_or_sse(text: &str, sse_enabled: bool) -> bool {
    let first_line = text.lines().next().map(str::trim).unwrap_or_default();
    starts_like_http_message(first_line) || (sse_enabled && starts_like_sse(first_line))
}

pub(super) fn starts_like_http_message(first_line: &str) -> bool {
    if first_line.starts_with("HTTP/") {
        return true;
    }
    let mut parts = first_line.split_whitespace();
    let Some(method) = parts.next() else {
        return false;
    };
    let Some(_) = parts.next() else {
        return false;
    };
    let Some(version) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && method
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'-')
        && version.starts_with("HTTP/")
}

pub(super) fn header_prefix_len(text: &str) -> Option<usize> {
    header_boundary(text).map(|(header_end, separator_len)| header_end + separator_len)
}

pub(super) fn take_message(
    text: &mut String,
    config: &ApplicationProtocolConfig,
    summary_only: bool,
) -> Result<Option<HttpMessage>, String> {
    if text
        .lines()
        .next()
        .map(str::trim)
        .is_some_and(starts_like_sse)
    {
        if summary_only || !config.sse_enabled {
            text.clear();
        }
        return Ok(None);
    }
    let Some((header_end, separator_len)) = header_boundary(text) else {
        return Ok(None);
    };
    let header_text = &text[..header_end];
    let headers = parse_headers(header_text)?;
    let body_start = header_end + separator_len;
    if summary_only {
        let message = HttpMessage {
            first_line: headers.first_line,
            fields: headers.fields,
            body: String::new(),
        };
        text.clear();
        return Ok(Some(message));
    }
    let (body, consumed) = if let Some(length) = headers.content_length {
        let message_end = body_start
            .checked_add(length)
            .ok_or_else(|| "application HTTP content length overflow".to_string())?;
        if text.len() < message_end {
            return Ok(None);
        }
        (text[body_start..message_end].to_string(), message_end)
    } else if headers.is_chunked() {
        if config.sse_enabled {
            let Some((body, chunked_len)) = parse_chunked_body(&text[body_start..])? else {
                return Ok(None);
            };
            (body, body_start + chunked_len)
        } else {
            (String::new(), text.len())
        }
    } else {
        (String::new(), body_start)
    };

    let message = HttpMessage {
        first_line: headers.first_line,
        fields: headers.fields,
        body,
    };
    text.drain(..consumed);
    Ok(Some(message))
}

pub(super) fn take_chunked_sse_head(
    text: &mut String,
    config: &ApplicationProtocolConfig,
) -> Result<Option<HttpMessage>, String> {
    if !config.sse_enabled {
        return Ok(None);
    }
    let Some((header_end, separator_len)) = header_boundary(text) else {
        return Ok(None);
    };
    let headers = parse_headers(&text[..header_end])?;
    if !headers.is_chunked() {
        return Ok(None);
    }
    let body_start = header_end + separator_len;
    if text[body_start..]
        .lines()
        .next()
        .map(str::trim)
        .is_some_and(starts_like_sse)
    {
        return Ok(None);
    }
    let message = HttpMessage {
        first_line: headers.first_line,
        fields: headers.fields,
        body: String::new(),
    };
    if !message.is_sse() {
        return Ok(None);
    }
    text.drain(..body_start);
    Ok(Some(message))
}

pub(super) fn take_streaming_sse_events(
    text: &mut String,
    config: &ApplicationProtocolConfig,
) -> Result<Vec<ApplicationPayload>, String> {
    if !config.sse_enabled
        || !text
            .lines()
            .next()
            .map(str::trim)
            .is_some_and(starts_like_sse)
    {
        return Ok(Vec::new());
    }
    streaming::take_complete_sse_events(text, config)
}

pub(super) struct ChunkedSseDrain {
    pub(super) payloads: Vec<ApplicationPayload>,
    pub(super) done: bool,
}

pub(super) fn take_chunked_sse_events(
    text: &mut String,
    pending_sse: &mut String,
    config: &ApplicationProtocolConfig,
) -> Result<ChunkedSseDrain, String> {
    let chunked = streaming::drain_complete_chunks(text)?;
    for body in chunked.bodies {
        pending_sse.push_str(&body);
    }
    let mut payloads = streaming::take_complete_sse_events(pending_sse, config)?;
    if chunked.done && !pending_sse.is_empty() {
        payloads.extend(streaming::sse_event_payloads(pending_sse, config)?);
        pending_sse.clear();
    }
    Ok(ChunkedSseDrain {
        payloads,
        done: chunked.done,
    })
}

pub(super) struct HttpMessage {
    first_line: String,
    fields: BTreeMap<String, String>,
    body: String,
}

impl HttpMessage {
    pub(super) fn to_payload(
        &self,
        segment: &PayloadSegment,
        config: &ApplicationProtocolConfig,
        semantic_retention: &SemanticRetentionConfig,
        consumed_by_llm: bool,
    ) -> ApplicationPayload {
        let mut metadata = BTreeMap::from([
            (
                "direction".to_string(),
                format!("{:?}", segment.direction).to_lowercase(),
            ),
            (
                "source_boundary".to_string(),
                format!("{:?}", segment.source_boundary),
            ),
            ("stream_key".to_string(), segment.stream_key.to_string()),
            ("payload_sequence".to_string(), segment.sequence.to_string()),
            (
                "payload_segment_id".to_string(),
                segment.segment_id.get().to_string(),
            ),
        ]);
        add_headers(
            &mut metadata,
            &self.fields,
            config,
            semantic_retention.http_headers(),
        );
        add_body(
            &mut metadata,
            &self.body,
            semantic_retention.http_body_content_for_http_message(consumed_by_llm),
        );
        if let Some(status) = self.response_status() {
            metadata.insert("status_code".to_string(), status.code);
            if let Some(reason) = status.reason {
                metadata.insert("reason".to_string(), reason);
            }
            return ApplicationPayload {
                protocol: status.version,
                operation: "response".to_string(),
                summary: status.summary,
                metadata,
            };
        }
        if let Some(request) = self.request_line() {
            metadata.insert("method".to_string(), request.method.clone());
            metadata.insert("target".to_string(), request.target.clone());
            return ApplicationPayload {
                protocol: request.version,
                operation: "request".to_string(),
                summary: format!("{} {}", request.method, request.target),
                metadata,
            };
        }
        ApplicationPayload {
            protocol: "http/1.x".to_string(),
            operation: "message".to_string(),
            summary: self.first_line.clone(),
            metadata,
        }
    }

    pub(super) fn is_sse(&self) -> bool {
        self.fields
            .get("content-type")
            .map(|value| value.to_ascii_lowercase().contains("text/event-stream"))
            .unwrap_or(false)
            || self.body.lines().any(|line| line.starts_with("data:"))
    }

    pub(super) fn sse_events(
        &self,
        config: &ApplicationProtocolConfig,
    ) -> Result<Vec<ApplicationPayload>, String> {
        streaming::sse_event_payloads(&self.body, config)
    }

    fn request_line(&self) -> Option<RequestLine> {
        let mut parts = self.first_line.split_whitespace();
        let method = parts.next()?;
        let target = parts.next()?;
        let version = parts.next()?;
        if !version.starts_with("HTTP/") {
            return None;
        }
        Some(RequestLine {
            method: method.to_string(),
            target: target.to_string(),
            version: version.to_ascii_lowercase(),
        })
    }

    fn response_status(&self) -> Option<ResponseStatus> {
        let mut parts = self.first_line.splitn(3, ' ');
        let version = parts.next()?;
        if !version.starts_with("HTTP/") {
            return None;
        }
        let code = parts.next()?.to_string();
        let reason = parts.next().map(str::to_string);
        let summary = reason
            .as_ref()
            .map(|reason| format!("{code} {reason}"))
            .unwrap_or_else(|| code.clone());
        Some(ResponseStatus {
            version: version.to_ascii_lowercase(),
            code,
            reason,
            summary,
        })
    }
}

struct ParsedHeaders {
    first_line: String,
    fields: BTreeMap<String, String>,
    content_length: Option<usize>,
}

impl ParsedHeaders {
    fn is_chunked(&self) -> bool {
        self.fields
            .get("transfer-encoding")
            .map(|value| value.eq_ignore_ascii_case("chunked"))
            .unwrap_or(false)
    }
}

struct RequestLine {
    method: String,
    target: String,
    version: String,
}

struct ResponseStatus {
    version: String,
    code: String,
    reason: Option<String>,
    summary: String,
}

fn header_boundary(text: &str) -> Option<(usize, usize)> {
    match (text.find("\r\n\r\n"), text.find("\n\n")) {
        (Some(crlf), Some(lf)) if crlf <= lf => Some((crlf, "\r\n\r\n".len())),
        (Some(_), Some(lf)) => Some((lf, "\n\n".len())),
        (Some(crlf), None) => Some((crlf, "\r\n\r\n".len())),
        (None, Some(lf)) => Some((lf, "\n\n".len())),
        (None, None) => None,
    }
}

fn parse_headers(text: &str) -> Result<ParsedHeaders, String> {
    let mut lines = text.lines();
    let first_line = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| "application HTTP message missing first line".to_string())?
        .to_string();
    let mut fields = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        fields.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let content_length = fields
        .get("content-length")
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|error| format!("invalid HTTP content-length: {error}"))
        })
        .transpose()?;
    Ok(ParsedHeaders {
        first_line,
        fields,
        content_length,
    })
}

fn parse_chunked_body(text: &str) -> Result<Option<(String, usize)>, String> {
    if text
        .lines()
        .next()
        .map(str::trim)
        .is_some_and(starts_like_sse)
    {
        return Ok(Some((text.to_string(), text.len())));
    }
    let mut remaining = text;
    let mut consumed = 0;
    let mut body = String::new();
    loop {
        let Some(line_end) = remaining.find("\r\n") else {
            return Ok(None);
        };
        let size_line = &remaining[..line_end];
        let size_text = size_line.split(';').next().unwrap_or(size_line).trim();
        let Ok(size) = usize::from_str_radix(size_text, 16) else {
            return Ok(Some((text.to_string(), text.len())));
        };
        let data_start = line_end + "\r\n".len();
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(|| "HTTP chunk size overflow".to_string())?;
        let chunk_end = data_end
            .checked_add("\r\n".len())
            .ok_or_else(|| "HTTP chunk terminator overflow".to_string())?;
        if remaining.len() < chunk_end {
            return Ok(None);
        }
        if size == 0 {
            consumed += chunk_end;
            return Ok(Some((body, consumed)));
        }
        body.push_str(&remaining[data_start..data_end]);
        consumed += chunk_end;
        remaining = &remaining[chunk_end..];
    }
}

fn starts_like_sse(line: &str) -> bool {
    line.starts_with("event:") || line.starts_with("data:")
}

fn add_headers(
    metadata: &mut BTreeMap<String, String>,
    fields: &BTreeMap<String, String>,
    config: &ApplicationProtocolConfig,
    retention: HttpHeadersRetention,
) {
    match retention {
        HttpHeadersRetention::None => return,
        HttpHeadersRetention::Metadata => add_selected_headers(metadata, fields, config),
        HttpHeadersRetention::Full => {
            let headers = fields
                .iter()
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect::<Map<_, _>>();
            metadata.insert(
                "http.headers_json".to_string(),
                Value::Object(headers).to_string(),
            );
        }
    }
}

fn add_selected_headers(
    metadata: &mut BTreeMap<String, String>,
    fields: &BTreeMap<String, String>,
    config: &ApplicationProtocolConfig,
) {
    if config.capture_host
        && let Some(host) = fields.get("host")
    {
        metadata.insert("host".to_string(), host.clone());
    }
    for key in ["content-type", "transfer-encoding", "content-length"] {
        if let Some(value) = fields.get(key) {
            metadata.insert(key.replace('-', "_"), value.clone());
        }
    }
}

fn add_body(metadata: &mut BTreeMap<String, String>, body: &str, retention: HttpBodyRetention) {
    if body.is_empty() {
        return;
    }
    match retention {
        HttpBodyRetention::None => {}
        HttpBodyRetention::Text => {
            metadata.insert("http.body_text".to_string(), body.to_string());
        }
        HttpBodyRetention::Json => {
            if let Ok(value) = serde_json::from_str::<Value>(body) {
                metadata.insert("http.body_json".to_string(), value.to_string());
                metadata.insert("http.body_json_state".to_string(), "valid".to_string());
            } else {
                metadata.insert(
                    "http.body_json_state".to_string(),
                    "invalid_or_unavailable".to_string(),
                );
            }
        }
        HttpBodyRetention::Raw => {
            metadata.insert(
                "http.body_base64".to_string(),
                base64_encode(body.as_bytes()),
            );
        }
    }
}

#[cfg(test)]
#[path = "parser/tests.rs"]
mod tests;
