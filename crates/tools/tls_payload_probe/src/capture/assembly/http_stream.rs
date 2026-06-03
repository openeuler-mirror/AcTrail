//! Incremental HTTP body fragment extraction.

use std::collections::HashMap;

use crate::{ToolError, ToolResult};

use super::chunked::scan_available_chunks;
use super::http1::{content_length, header_boundary, is_chunked, is_plain_textual, parse_headers};
use super::model::{
    AssemblyKey, HttpAssemblyOutput, HttpBodyFragment, HttpBodyFragmentBody, ParsedHeaders,
};

#[derive(Clone, Debug, Default)]
pub(super) struct StreamingState {
    pub(super) emitted_body_bytes: usize,
    body_start: Option<usize>,
    wire_cursor: usize,
    chunk_data_end: Option<usize>,
    complete_wire_end: Option<usize>,
}

impl StreamingState {
    pub(super) fn complete_wire_end(&self) -> Option<usize> {
        self.complete_wire_end
    }

    fn started(&self) -> bool {
        self.body_start.is_some()
    }
}

pub(super) fn take_body_fragment(
    buffer: &[u8],
    key: AssemblyKey,
    state: &mut StreamingState,
) -> ToolResult<Option<HttpBodyFragment>> {
    let Some((header_end, separator_len)) = header_boundary(buffer) else {
        return Ok(None);
    };
    let header_text = String::from_utf8(buffer[..header_end].to_vec())
        .map_err(|error| ToolError::new(format!("HTTP header block is not UTF-8: {error}")))?;
    let headers = parse_headers(&header_text)?;
    let body_start = header_end
        .checked_add(separator_len)
        .ok_or_else(|| ToolError::new("HTTP body offset overflow"))?;
    if state.body_start.is_none() {
        state.body_start = Some(body_start);
        state.wire_cursor = body_start;
    }
    let Some(fragment) = available_body_fragment(buffer, &headers, state)? else {
        return Ok(None);
    };
    let body = if is_plain_textual(&headers.fields) {
        let text = String::from_utf8_lossy(&fragment).into_owned();
        HttpBodyFragmentBody::Text {
            bytes: text.len(),
            text,
            data: fragment,
        }
    } else {
        HttpBodyFragmentBody::Binary {
            bytes: fragment.len(),
            data: fragment,
        }
    };
    Ok(Some(HttpBodyFragment {
        pid: key.pid,
        stream_key: key.stream_key,
        direction: key.direction,
        first_line: headers.first_line,
        headers: headers.headers,
        body,
    }))
}

pub(super) fn stream_partial_body(
    buffer: &[u8],
    key: AssemblyKey,
    streams: &mut HashMap<AssemblyKey, StreamingState>,
) -> ToolResult<Option<HttpAssemblyOutput>> {
    let state = streams.entry(key).or_default();
    let was_started = state.started();
    let Some(fragment) = take_body_fragment(buffer, key, state)? else {
        if !was_started && !state.started() {
            streams.remove(&key);
        }
        return Ok(None);
    };
    Ok(Some(HttpAssemblyOutput::BodyFragment(fragment)))
}

fn available_body_fragment(
    buffer: &[u8],
    headers: &ParsedHeaders,
    state: &mut StreamingState,
) -> ToolResult<Option<Vec<u8>>> {
    if let Some(length) = content_length(&headers.fields)? {
        let body_start = state
            .body_start
            .ok_or_else(|| ToolError::new("HTTP streaming state has no body start"))?;
        let body_end = body_start
            .checked_add(length)
            .ok_or_else(|| ToolError::new("HTTP content-length overflow"))?;
        let available_end = buffer.len().min(body_end);
        if available_end <= state.wire_cursor {
            return Ok(None);
        }
        let fragment = buffer[state.wire_cursor..available_end].to_vec();
        state.wire_cursor = available_end;
        state.emitted_body_bytes = state
            .emitted_body_bytes
            .checked_add(fragment.len())
            .ok_or_else(|| ToolError::new("HTTP streamed body length overflow"))?;
        if state.wire_cursor == body_end {
            state.complete_wire_end = Some(body_end);
        }
        return Ok(Some(fragment));
    }
    if is_chunked(&headers.fields) {
        let scan = scan_available_chunks(buffer, state.wire_cursor, state.chunk_data_end)?;
        state.wire_cursor = scan.cursor;
        state.chunk_data_end = scan.chunk_data_end;
        if let Some(complete_end) = scan.complete_end {
            state.complete_wire_end = Some(complete_end);
        }
        if scan.body.is_empty() {
            return Ok(None);
        }
        state.emitted_body_bytes = state
            .emitted_body_bytes
            .checked_add(scan.body.len())
            .ok_or_else(|| ToolError::new("HTTP streamed body length overflow"))?;
        return Ok(Some(scan.body));
    }
    Ok(None)
}
