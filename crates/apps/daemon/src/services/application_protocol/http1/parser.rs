//! HTTP/1.x and SSE parsing helpers for application semantic analysis.

use std::collections::{BTreeMap, VecDeque};

use config_core::daemon::{ApplicationProtocolConfig, SseDataPolicy};
use model_core::event::ApplicationPayload;
use model_core::payload::PayloadSegment;

pub(super) fn starts_like_http_or_sse(text: &str, sse_enabled: bool) -> bool {
    let first_line = text.lines().next().map(str::trim).unwrap_or_default();
    first_line.starts_with("HTTP/")
        || (first_line.contains(" HTTP/") && first_line.split_whitespace().count() >= 3)
        || (sse_enabled && starts_like_sse(first_line))
}

pub(super) fn take_message(
    text: &mut String,
    config: &ApplicationProtocolConfig,
) -> Result<Option<HttpMessage>, String> {
    if text
        .lines()
        .next()
        .map(str::trim)
        .is_some_and(starts_like_sse)
    {
        if !config.sse_enabled {
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
        add_selected_headers(&mut metadata, &self.fields, config);
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
        let mut output = Vec::new();
        for block in sse_blocks(&self.body) {
            let fields = parse_sse_block(block);
            if fields.is_empty() {
                continue;
            }
            let event_name = fields
                .get("event")
                .cloned()
                .unwrap_or_else(|| "message".to_string());
            let mut metadata = BTreeMap::from([("event".to_string(), event_name.clone())]);
            if let Some(data) = fields.get("data") {
                metadata.insert("data_size".to_string(), data.len().to_string());
                if matches!(config.sse_data_policy, SseDataPolicy::Preview) {
                    let (preview, truncated) = preview_data(data, config.sse_max_data_bytes)?;
                    metadata.insert("data_preview".to_string(), preview);
                    metadata.insert("data_truncated".to_string(), truncated.to_string());
                }
            }
            output.push(ApplicationPayload {
                protocol: "sse".to_string(),
                operation: "event".to_string(),
                summary: event_name,
                metadata,
            });
        }
        Ok(output)
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

fn sse_blocks(body: &str) -> VecDeque<&str> {
    let mut blocks = VecDeque::new();
    for block in body.split("\n\n") {
        let normalized = block.trim_matches(['\r', '\n']);
        if !normalized.is_empty() {
            blocks.push_back(normalized);
        }
    }
    blocks
}

fn parse_sse_block(block: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::<String, String>::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let key = name.trim().to_ascii_lowercase();
        let value = value.trim_start();
        fields
            .entry(key)
            .and_modify(|existing| {
                existing.push('\n');
                existing.push_str(value);
            })
            .or_insert_with(|| value.to_string());
    }
    fields
}

fn preview_data(data: &str, max_bytes: u64) -> Result<(String, bool), String> {
    let max_bytes = usize::try_from(max_bytes).map_err(|error| error.to_string())?;
    if data.len() <= max_bytes {
        return Ok((data.to_string(), false));
    }
    let mut end = max_bytes;
    while !data.is_char_boundary(end) {
        end = end
            .checked_sub(1)
            .ok_or_else(|| "SSE preview boundary underflow".to_string())?;
    }
    Ok((data[..end].to_string(), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SSE_MAX_BUFFER_BYTES: u64 = 4096;
    const TEST_SSE_MAX_DATA_BYTES: u64 = 4096;
    const TEST_HTTP2_MAX_FRAME_BYTES: u64 = 16384;
    const TEST_HTTP2_MAX_CONNECTION_BUFFER_BYTES: u64 = 4096;
    const TEST_HTTP2_MAX_DATA_PREVIEW_BYTES: u64 = 4096;

    #[test]
    fn chunked_response_with_dechunked_sse_body_does_not_error_when_sse_is_disabled() {
        let mut text = claude_streaming_response_fragment();
        let message = take_message(&mut text, &test_config(false))
            .unwrap()
            .expect("HTTP response headers");

        assert_eq!(message.first_line, "HTTP/1.1 200 OK");
        assert!(message.body.is_empty());
        assert!(text.is_empty());
    }

    #[test]
    fn chunked_response_with_dechunked_sse_body_can_emit_sse_preview() {
        let config = test_config(true);
        let mut text = claude_streaming_response_fragment();
        let message = take_message(&mut text, &config)
            .unwrap()
            .expect("HTTP response headers");

        let events = message.sse_events(&config).unwrap();
        assert!(events.iter().any(|payload| {
            payload.operation == "event" && payload.summary == "content_block_delta"
        }));
        assert!(text.is_empty());
    }

    fn claude_streaming_response_fragment() -> String {
        concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/event-stream\r\n",
            "Transfer-Encoding: chunked\r\n",
            "\r\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0}\n\n",
        )
        .to_string()
    }

    fn test_config(sse_enabled: bool) -> ApplicationProtocolConfig {
        ApplicationProtocolConfig {
            enabled: true,
            http1_enabled: true,
            http2_enabled: false,
            capture_host: false,
            sse_enabled,
            sse_data_policy: if sse_enabled {
                SseDataPolicy::Preview
            } else {
                SseDataPolicy::Disabled
            },
            sse_max_buffer_bytes: TEST_SSE_MAX_BUFFER_BYTES,
            sse_max_data_bytes: TEST_SSE_MAX_DATA_BYTES,
            http2_max_frame_bytes: TEST_HTTP2_MAX_FRAME_BYTES,
            http2_max_connection_buffer_bytes: TEST_HTTP2_MAX_CONNECTION_BUFFER_BYTES,
            http2_emit_data_preview: false,
            http2_max_data_preview_bytes: TEST_HTTP2_MAX_DATA_PREVIEW_BYTES,
        }
    }
}
