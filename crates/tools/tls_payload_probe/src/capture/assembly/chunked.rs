//! HTTP chunked transfer body parsing.

use crate::{ToolError, ToolResult};

const HTTP_CRLF: &[u8] = b"\r\n";
const HTTP_LF: &[u8] = b"\n";
const HTTP_CRLF_HEADER_END: &[u8] = b"\r\n\r\n";
const HTTP_LF_HEADER_END: &[u8] = b"\n\n";

pub(super) fn parse_chunked_body(bytes: &[u8]) -> ToolResult<Option<(Vec<u8>, usize)>> {
    match parse_chunked(bytes)? {
        ChunkedBody::Complete { body, consumed } => Ok(Some((body, consumed))),
        ChunkedBody::Partial { .. } => Ok(None),
    }
}

pub(super) fn parse_available_chunked_body(bytes: &[u8]) -> ToolResult<Vec<u8>> {
    match parse_chunked(bytes)? {
        ChunkedBody::Complete { body, .. } | ChunkedBody::Partial { body } => Ok(body),
    }
}

pub(super) struct ChunkedScan {
    pub(super) body: Vec<u8>,
    pub(super) cursor: usize,
    pub(super) chunk_data_end: Option<usize>,
    pub(super) complete_end: Option<usize>,
}

pub(super) fn scan_available_chunks(
    bytes: &[u8],
    mut cursor: usize,
    mut chunk_data_end: Option<usize>,
) -> ToolResult<ChunkedScan> {
    let mut body = Vec::new();
    loop {
        if let Some(data_end) = chunk_data_end {
            let available_end = bytes.len().min(data_end);
            if available_end > cursor {
                body.extend_from_slice(&bytes[cursor..available_end]);
                cursor = available_end;
            }
            if cursor < data_end {
                return Ok(ChunkedScan {
                    body,
                    cursor,
                    chunk_data_end,
                    complete_end: None,
                });
            }
            let Some(chunk_end) = chunk_data_end_marker(bytes, data_end) else {
                return Ok(ChunkedScan {
                    body,
                    cursor,
                    chunk_data_end,
                    complete_end: None,
                });
            };
            cursor = chunk_end;
            chunk_data_end = None;
            continue;
        }
        let Some((line_end, separator_len)) = line_boundary(&bytes[cursor..]) else {
            return Ok(ChunkedScan {
                body,
                cursor,
                chunk_data_end,
                complete_end: None,
            });
        };
        let size_line = &bytes[cursor..cursor + line_end];
        let size_text = chunk_size_text(size_line)?;
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|error| ToolError::new(format!("invalid HTTP chunk size: {error}")))?;
        let data_start = cursor
            .checked_add(line_end)
            .and_then(|value| value.checked_add(separator_len))
            .ok_or_else(|| ToolError::new("HTTP chunk data offset overflow"))?;
        if size == 0 {
            let Some(consumed) = chunk_trailer_len(&bytes[data_start..])? else {
                return Ok(ChunkedScan {
                    body,
                    cursor,
                    chunk_data_end,
                    complete_end: None,
                });
            };
            let complete_end = data_start
                .checked_add(consumed)
                .ok_or_else(|| ToolError::new("HTTP chunk trailer offset overflow"))?;
            return Ok(ChunkedScan {
                body,
                cursor: complete_end,
                chunk_data_end: None,
                complete_end: Some(complete_end),
            });
        }
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(|| ToolError::new("HTTP chunk data length overflow"))?;
        cursor = data_start;
        chunk_data_end = Some(data_end);
    }
}

enum ChunkedBody {
    Complete { body: Vec<u8>, consumed: usize },
    Partial { body: Vec<u8> },
}

fn parse_chunked(bytes: &[u8]) -> ToolResult<ChunkedBody> {
    let mut cursor = 0;
    let mut body = Vec::new();
    loop {
        let Some((line_end, separator_len)) = line_boundary(&bytes[cursor..]) else {
            return Ok(ChunkedBody::Partial { body });
        };
        let size_line = &bytes[cursor..cursor + line_end];
        let size_text = chunk_size_text(size_line)?;
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|error| ToolError::new(format!("invalid HTTP chunk size: {error}")))?;
        let data_start = cursor
            .checked_add(line_end)
            .and_then(|value| value.checked_add(separator_len))
            .ok_or_else(|| ToolError::new("HTTP chunk data offset overflow"))?;
        if size == 0 {
            let Some(consumed) = chunk_trailer_len(&bytes[data_start..])? else {
                return Ok(ChunkedBody::Partial { body });
            };
            return Ok(ChunkedBody::Complete {
                body,
                consumed: data_start + consumed,
            });
        }
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(|| ToolError::new("HTTP chunk data length overflow"))?;
        if bytes.len() < data_end {
            body.extend_from_slice(&bytes[data_start..]);
            return Ok(ChunkedBody::Partial { body });
        }
        let Some(chunk_end) = chunk_data_end_marker(bytes, data_end) else {
            body.extend_from_slice(&bytes[data_start..data_end]);
            return Ok(ChunkedBody::Partial { body });
        };
        body.extend_from_slice(&bytes[data_start..data_end]);
        cursor = chunk_end;
    }
}

fn chunk_size_text(bytes: &[u8]) -> ToolResult<&str> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| ToolError::new(format!("HTTP chunk size is not UTF-8: {error}")))?;
    Ok(text.split(';').next().unwrap_or(text).trim())
}

fn chunk_data_end_marker(bytes: &[u8], data_end: usize) -> Option<usize> {
    if bytes.get(data_end..data_end + HTTP_CRLF.len()) == Some(HTTP_CRLF) {
        return Some(data_end + HTTP_CRLF.len());
    }
    if bytes.get(data_end..data_end + HTTP_LF.len()) == Some(HTTP_LF) {
        return Some(data_end + HTTP_LF.len());
    }
    None
}

fn chunk_trailer_len(bytes: &[u8]) -> ToolResult<Option<usize>> {
    if bytes.starts_with(HTTP_CRLF) {
        return Ok(Some(HTTP_CRLF.len()));
    }
    if bytes.starts_with(HTTP_LF) {
        return Ok(Some(HTTP_LF.len()));
    }
    Ok(header_boundary(bytes).map(|(header_end, separator_len)| header_end + separator_len))
}

fn header_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
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

fn line_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    match (find_bytes(bytes, HTTP_CRLF), find_bytes(bytes, HTTP_LF)) {
        (Some(crlf), Some(lf)) if crlf <= lf => Some((crlf, HTTP_CRLF.len())),
        (Some(_), Some(lf)) => Some((lf, HTTP_LF.len())),
        (Some(crlf), None) => Some((crlf, HTTP_CRLF.len())),
        (None, Some(lf)) => Some((lf, HTTP_LF.len())),
        (None, None) => None,
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
