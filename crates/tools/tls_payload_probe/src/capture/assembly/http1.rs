//! HTTP/1.x stream assembly.

use std::collections::{BTreeMap, HashMap};

use crate::capture::{CaptureConfig, CaptureEvent};
use crate::{ToolError, ToolResult};

use super::chunked::{parse_available_chunked_body, parse_chunked_body};
use super::decode::body_content;
use super::http_stream::{StreamingState, stream_partial_body, take_body_fragment};
use super::model::{
    AssembledHttp, AssemblyConfig, AssemblyKey, HttpAssemblyOutput, HttpBody, HttpHeader,
    ParsedHeaders,
};

const HTTP_CRLF_HEADER_END: &[u8] = b"\r\n\r\n";
const HTTP_LF_HEADER_END: &[u8] = b"\n\n";
const HTTP_VERSION_PREFIX: &[u8] = b"HTTP/";
const HTTP_VERSION_MARKER: &str = " HTTP/";
const HEADER_CONTENT_LENGTH: &str = "content-length";
const HEADER_CONTENT_TYPE: &str = "content-type";
const HEADER_CONTENT_ENCODING: &str = "content-encoding";
const HEADER_TRANSFER_ENCODING: &str = "transfer-encoding";
const TRANSFER_CHUNKED: &str = "chunked";
const CONTENT_TYPE_APPLICATION_JSON: &str = "application/json";
const CONTENT_TYPE_JSON_SUFFIX: &str = "+json";
const CONTENT_TYPE_TEXT_PREFIX: &str = "text/";
const ENCODING_IDENTITY: &str = "identity";
const PARTIAL_BODY_REASON: &str = "target exited before complete HTTP body was assembled";
const HTTP_REQUEST_METHODS: &[&[u8]] = &[
    b"GET ",
    b"POST ",
    b"PUT ",
    b"PATCH ",
    b"DELETE ",
    b"HEAD ",
    b"OPTIONS ",
    b"CONNECT ",
    b"TRACE ",
];

#[derive(Clone, Debug)]
pub(crate) struct HttpAssembler {
    config: AssemblyConfig,
    buffers: HashMap<AssemblyKey, Vec<u8>>,
    streams: HashMap<AssemblyKey, StreamingState>,
}

impl HttpAssembler {
    pub(crate) fn new(config: &CaptureConfig) -> Self {
        Self {
            config: AssemblyConfig::from(config),
            buffers: HashMap::new(),
            streams: HashMap::new(),
        }
    }

    pub(crate) fn push(&mut self, event: &CaptureEvent) -> ToolResult<Vec<HttpAssemblyOutput>> {
        if event.flags.truncated {
            return Err(ToolError::new(format!(
                "cannot assemble truncated TLS payload event pid={} tid={} stream=0x{:x}",
                event.pid, event.tid, event.stream_key
            )));
        }
        let key = AssemblyKey::from_event(event);
        let buffer = self.buffers.entry(key).or_default();
        buffer.extend_from_slice(&event.captured);
        if buffer.len() > self.config.max_buffer_bytes {
            return Err(ToolError::new(format!(
                "HTTP assembly buffer exceeded {} bytes for pid={} stream=0x{:x} direction={}",
                self.config.max_buffer_bytes,
                key.pid,
                key.stream_key,
                key.direction.as_str()
            )));
        }
        take_outputs(buffer, key, &self.config, &mut self.streams)
    }

    pub(crate) fn finish(&mut self) -> ToolResult<Vec<HttpAssemblyOutput>> {
        let mut output = Vec::new();
        for (key, buffer) in &mut self.buffers {
            output.extend(take_outputs(buffer, *key, &self.config, &mut self.streams)?);
            if starts_like_http1(buffer) {
                if let Some(mut message) = partial_message(buffer, *key, &self.config)? {
                    if let Some(state) = self.streams.get(key)
                        && state.emitted_body_bytes > 0
                    {
                        message.body = HttpBody::Streamed {
                            bytes: state.emitted_body_bytes,
                        };
                    }
                    output.push(HttpAssemblyOutput::Message(message));
                }
                buffer.clear();
            }
            self.streams.remove(key);
        }
        Ok(output)
    }
}

fn take_outputs(
    buffer: &mut Vec<u8>,
    key: AssemblyKey,
    config: &AssemblyConfig,
    streams: &mut HashMap<AssemblyKey, StreamingState>,
) -> ToolResult<Vec<HttpAssemblyOutput>> {
    let mut output = Vec::new();
    loop {
        if buffer.is_empty() {
            return Ok(output);
        }
        if !starts_like_http1(buffer) {
            buffer.clear();
            streams.remove(&key);
            return Ok(output);
        }
        let had_streamed_body = streams
            .get(&key)
            .map(|state| state.emitted_body_bytes > 0)
            .unwrap_or(false);
        if had_streamed_body {
            if let Some(state) = streams.get_mut(&key)
                && let Some(fragment) = take_body_fragment(buffer, key, state)?
            {
                output.push(HttpAssemblyOutput::BodyFragment(fragment));
            }
            if let Some(state) = streams.get(&key)
                && let Some(consumed) = state.complete_wire_end()
            {
                let message = streamed_message(buffer, key, state.emitted_body_bytes)?;
                buffer.drain(..consumed);
                streams.remove(&key);
                output.push(HttpAssemblyOutput::Message(message));
                continue;
            }
        }
        let streamed_bytes = streams
            .get(&key)
            .map(|state| state.emitted_body_bytes)
            .unwrap_or_default();
        let Some(mut message) = take_message(buffer, key, config)? else {
            if !had_streamed_body && let Some(fragment) = stream_partial_body(buffer, key, streams)?
            {
                output.push(fragment);
            }
            return Ok(output);
        };
        if streamed_bytes > 0 {
            message.body = HttpBody::Streamed {
                bytes: streamed_bytes,
            };
        }
        streams.remove(&key);
        output.push(HttpAssemblyOutput::Message(message));
    }
}

fn take_message(
    buffer: &mut Vec<u8>,
    key: AssemblyKey,
    config: &AssemblyConfig,
) -> ToolResult<Option<AssembledHttp>> {
    if buffer.is_empty() {
        return Ok(None);
    }
    if !starts_like_http1(buffer) {
        buffer.clear();
        return Ok(None);
    }
    let Some((header_end, separator_len)) = header_boundary(buffer) else {
        return Ok(None);
    };
    let header_text = String::from_utf8(buffer[..header_end].to_vec())
        .map_err(|error| ToolError::new(format!("HTTP header block is not UTF-8: {error}")))?;
    let headers = parse_headers(&header_text)?;
    let body_start = header_end
        .checked_add(separator_len)
        .ok_or_else(|| ToolError::new("HTTP body offset overflow"))?;
    let (body, consumed) = take_body(buffer, body_start, &headers)?;
    let Some(consumed) = consumed else {
        return Ok(None);
    };
    let body = body_content(body, &headers.fields, &config.decode)?;
    let message = AssembledHttp {
        pid: key.pid,
        stream_key: key.stream_key,
        direction: key.direction,
        first_line: headers.first_line,
        headers: headers.headers,
        body,
    };
    buffer.drain(..consumed);
    Ok(Some(message))
}

fn streamed_message(
    buffer: &[u8],
    key: AssemblyKey,
    streamed_bytes: usize,
) -> ToolResult<AssembledHttp> {
    let Some((header_end, separator_len)) = header_boundary(buffer) else {
        return Err(ToolError::new(
            "cannot finish streamed HTTP message without headers",
        ));
    };
    let header_text = String::from_utf8(buffer[..header_end].to_vec())
        .map_err(|error| ToolError::new(format!("HTTP header block is not UTF-8: {error}")))?;
    let headers = parse_headers(&header_text)?;
    let _body_start = header_end
        .checked_add(separator_len)
        .ok_or_else(|| ToolError::new("HTTP body offset overflow"))?;
    Ok(AssembledHttp {
        pid: key.pid,
        stream_key: key.stream_key,
        direction: key.direction,
        first_line: headers.first_line,
        headers: headers.headers,
        body: HttpBody::Streamed {
            bytes: streamed_bytes,
        },
    })
}

fn partial_message(
    buffer: &[u8],
    key: AssemblyKey,
    config: &AssemblyConfig,
) -> ToolResult<Option<AssembledHttp>> {
    let Some((header_end, separator_len)) = header_boundary(buffer) else {
        return Ok(None);
    };
    let header_text = String::from_utf8(buffer[..header_end].to_vec())
        .map_err(|error| ToolError::new(format!("HTTP header block is not UTF-8: {error}")))?;
    let headers = parse_headers(&header_text)?;
    let body_start = header_end
        .checked_add(separator_len)
        .ok_or_else(|| ToolError::new("HTTP body offset overflow"))?;
    let buffered_bytes = buffer.len().saturating_sub(body_start);
    let body = partial_body(&buffer[body_start..], &headers, config, buffered_bytes)?;
    Ok(Some(AssembledHttp {
        pid: key.pid,
        stream_key: key.stream_key,
        direction: key.direction,
        first_line: headers.first_line,
        headers: headers.headers,
        body,
    }))
}

fn partial_body(
    bytes: &[u8],
    headers: &ParsedHeaders,
    config: &AssemblyConfig,
    buffered_bytes: usize,
) -> ToolResult<HttpBody> {
    if !is_chunked(&headers.fields) {
        return Ok(partial(buffered_bytes));
    }
    let body = parse_available_chunked_body(bytes)?;
    if body.is_empty() {
        if is_plain_textual(&headers.fields) && !bytes.is_empty() {
            return Ok(partial_text(bytes, buffered_bytes));
        }
        return Ok(partial(buffered_bytes));
    }
    if is_plain_textual(&headers.fields) {
        return Ok(partial_text(&body, buffered_bytes));
    }
    match body_content(body, &headers.fields, &config.decode)? {
        HttpBody::Text { bytes, text } => Ok(HttpBody::PartialText {
            bytes,
            buffered_bytes,
            reason: PARTIAL_BODY_REASON.to_string(),
            text,
        }),
        HttpBody::DecodedText {
            encoding,
            compressed_bytes,
            decoded_bytes,
            text,
        } => Ok(HttpBody::PartialDecodedText {
            encoding,
            compressed_bytes,
            decoded_bytes,
            buffered_bytes,
            reason: PARTIAL_BODY_REASON.to_string(),
            text,
        }),
        _ => Ok(partial(buffered_bytes)),
    }
}

fn partial(buffered_bytes: usize) -> HttpBody {
    HttpBody::Partial {
        buffered_bytes,
        reason: PARTIAL_BODY_REASON.to_string(),
    }
}

fn partial_text(bytes: &[u8], buffered_bytes: usize) -> HttpBody {
    let text = String::from_utf8_lossy(bytes).into_owned();
    HttpBody::PartialText {
        bytes: text.len(),
        buffered_bytes,
        reason: PARTIAL_BODY_REASON.to_string(),
        text,
    }
}

fn take_body(
    buffer: &[u8],
    body_start: usize,
    headers: &ParsedHeaders,
) -> ToolResult<(Vec<u8>, Option<usize>)> {
    if let Some(length) = content_length(&headers.fields)? {
        let body_end = body_start
            .checked_add(length)
            .ok_or_else(|| ToolError::new("HTTP content-length overflow"))?;
        if buffer.len() < body_end {
            return Ok((Vec::new(), None));
        }
        return Ok((buffer[body_start..body_end].to_vec(), Some(body_end)));
    }
    if is_chunked(&headers.fields) {
        let Some((body, body_len)) = parse_chunked_body(&buffer[body_start..])? else {
            return Ok((Vec::new(), None));
        };
        let consumed = body_start
            .checked_add(body_len)
            .ok_or_else(|| ToolError::new("HTTP chunked body length overflow"))?;
        return Ok((body, Some(consumed)));
    }
    Ok((Vec::new(), Some(body_start)))
}

pub(super) fn parse_headers(text: &str) -> ToolResult<ParsedHeaders> {
    let mut lines = text.lines();
    let first_line = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| ToolError::new("HTTP message missing first line"))?
        .to_string();
    if !first_line.starts_with("HTTP/") && !first_line.contains(HTTP_VERSION_MARKER) {
        return Err(ToolError::new(format!(
            "not an HTTP/1.x first line: {first_line}"
        )));
    }
    let mut headers = Vec::new();
    let mut fields = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let header = HttpHeader {
            name: name.trim().to_string(),
            value: value.trim().to_string(),
        };
        fields.insert(header.name.to_ascii_lowercase(), header.value.clone());
        headers.push(header);
    }
    Ok(ParsedHeaders {
        first_line,
        headers,
        fields,
    })
}

pub(super) fn content_length(fields: &BTreeMap<String, String>) -> ToolResult<Option<usize>> {
    fields
        .get(HEADER_CONTENT_LENGTH)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|error| ToolError::new(format!("invalid HTTP content-length: {error}")))
        })
        .transpose()
}

pub(super) fn is_chunked(fields: &BTreeMap<String, String>) -> bool {
    fields
        .get(HEADER_TRANSFER_ENCODING)
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(TRANSFER_CHUNKED))
        })
        .unwrap_or(false)
}

pub(super) fn is_plain_textual(fields: &BTreeMap<String, String>) -> bool {
    if fields
        .get(HEADER_CONTENT_ENCODING)
        .map(|value| {
            value
                .split(',')
                .any(|part| !part.trim().eq_ignore_ascii_case(ENCODING_IDENTITY))
        })
        .unwrap_or(false)
    {
        return false;
    }
    let Some(content_type) = fields.get(HEADER_CONTENT_TYPE) else {
        return false;
    };
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();
    media_type.starts_with(CONTENT_TYPE_TEXT_PREFIX)
        || media_type == CONTENT_TYPE_APPLICATION_JSON
        || media_type.ends_with(CONTENT_TYPE_JSON_SUFFIX)
}

fn starts_like_http1(bytes: &[u8]) -> bool {
    bytes.starts_with(HTTP_VERSION_PREFIX)
        || HTTP_REQUEST_METHODS
            .iter()
            .any(|method| bytes.starts_with(method))
}

pub(super) fn header_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    match (
        find_bytes(bytes, HTTP_CRLF_HEADER_END),
        find_bytes(bytes, HTTP_LF_HEADER_END),
    ) {
        (Some(crlf), Some(lf)) if crlf <= lf => Some((crlf, HTTP_CRLF_HEADER_END.len())),
        (Some(_), Some(lf)) => Some((lf, HTTP_LF_HEADER_END.len())),
        (Some(crlf), None) => Some((crlf, HTTP_CRLF_HEADER_END.len())),
        (None, Some(lf)) => Some((lf, HTTP_LF_HEADER_END.len())),
        (None, None) => None,
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
