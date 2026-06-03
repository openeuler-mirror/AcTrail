//! Streaming Server-Sent Events frame assembly.

use std::collections::HashMap;

use serde_json::Value;

use crate::capture::{CaptureConfig, CaptureDirection};
use crate::{ToolError, ToolResult};

use super::model::{AssembledHttp, AssemblyConfig, HttpBodyFragment};

const SSE_CRLF_FRAME_END: &str = "\r\n\r\n";
const SSE_LF_FRAME_END: &str = "\n\n";
const SSE_FIELD_EVENT: &str = "event";
const SSE_FIELD_DATA: &str = "data";
const SSE_CRLF_EVENT_START: &str = "\r\nevent:";
const SSE_LF_EVENT_START: &str = "\nevent:";
const SSE_CRLF_DATA_START: &str = "\r\ndata:";
const SSE_LF_DATA_START: &str = "\ndata:";
const JSON_OBJECT_START: &str = "{";
const SSE_DONE_DATA: &str = "[DONE]";

#[derive(Debug)]
pub(crate) struct SseAssembler {
    config: AssemblyConfig,
    buffers: HashMap<SseKey, String>,
}

impl SseAssembler {
    pub(crate) fn new(config: &CaptureConfig) -> Self {
        Self {
            config: AssemblyConfig::from(config),
            buffers: HashMap::new(),
        }
    }

    pub(crate) fn push_fragment(
        &mut self,
        fragment: &HttpBodyFragment,
    ) -> ToolResult<Vec<SseFrameEvent>> {
        if !fragment.is_event_stream() {
            return Ok(Vec::new());
        }
        let Some(text) = fragment.body_text() else {
            return Ok(Vec::new());
        };
        self.push_text(SseKey::from_fragment(fragment), text)
    }

    pub(crate) fn push_message(
        &mut self,
        message: &AssembledHttp,
    ) -> ToolResult<Vec<SseFrameEvent>> {
        if !message.is_event_stream() {
            return Ok(Vec::new());
        }
        let Some(text) = message.body_text() else {
            return Ok(Vec::new());
        };
        self.push_text(SseKey::from_message(message), text)
    }

    pub(crate) fn finish(&mut self) -> ToolResult<Vec<SseFrameEvent>> {
        let mut output = Vec::new();
        let keys = self.buffers.keys().copied().collect::<Vec<_>>();
        for key in keys {
            if let Some(buffer) = self.buffers.get_mut(&key) {
                output.extend(take_frames(buffer, key, false)?);
                if !buffer.trim().is_empty()
                    && let Some(frame) = parse_frame(key, buffer, false)?
                {
                    output.push(frame);
                }
            }
            self.buffers.remove(&key);
        }
        Ok(output)
    }

    fn push_text(&mut self, key: SseKey, text: &str) -> ToolResult<Vec<SseFrameEvent>> {
        let buffer = self.buffers.entry(key).or_default();
        buffer.push_str(text);
        if buffer.len() > self.config.max_buffer_bytes {
            return Err(ToolError::new(format!(
                "SSE assembly buffer exceeded {} bytes for pid={} stream=0x{:x} direction={}",
                self.config.max_buffer_bytes,
                key.pid,
                key.stream_key,
                key.direction.as_str()
            )));
        }
        take_frames(buffer, key, true)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SseFrameEvent {
    pub(crate) frame: SseFrame,
    pub(crate) data: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SseFrame {
    pub(crate) pid: u32,
    pub(crate) stream_key: u64,
    pub(crate) direction: CaptureDirection,
    pub(crate) event: Option<String>,
    pub(crate) data_bytes: usize,
    pub(crate) complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SseKey {
    pid: u32,
    stream_key: u64,
    direction: CaptureDirection,
}

impl SseKey {
    fn from_fragment(fragment: &HttpBodyFragment) -> Self {
        Self {
            pid: fragment.pid,
            stream_key: fragment.stream_key,
            direction: fragment.direction,
        }
    }

    fn from_message(message: &AssembledHttp) -> Self {
        Self {
            pid: message.pid,
            stream_key: message.stream_key,
            direction: message.direction,
        }
    }
}

fn take_frames(buffer: &mut String, key: SseKey, complete: bool) -> ToolResult<Vec<SseFrameEvent>> {
    let mut output = Vec::new();
    while let Some((frame_end, separator_len)) = frame_boundary(buffer) {
        let frame_text = buffer[..frame_end].to_string();
        buffer.drain(..frame_end + separator_len);
        if let Some(frame) = parse_frame(key, &frame_text, complete)? {
            output.push(frame);
        }
    }
    Ok(output)
}

fn frame_boundary(buffer: &str) -> Option<(usize, usize)> {
    let standard = match (
        buffer.find(SSE_CRLF_FRAME_END),
        buffer.find(SSE_LF_FRAME_END),
    ) {
        (Some(crlf), Some(lf)) if crlf <= lf => Some((crlf, SSE_CRLF_FRAME_END.len())),
        (Some(_), Some(lf)) => Some((lf, SSE_LF_FRAME_END.len())),
        (Some(crlf), None) => Some((crlf, SSE_CRLF_FRAME_END.len())),
        (None, Some(lf)) => Some((lf, SSE_LF_FRAME_END.len())),
        (None, None) => None,
    };
    let synthetic = earliest_boundary([
        synthetic_event_boundary(buffer),
        synthetic_data_boundary(buffer),
    ]);
    match (standard, synthetic) {
        (Some(standard), Some(synthetic)) if standard.0 <= synthetic.0 => Some(standard),
        (Some(_), Some(synthetic)) => Some(synthetic),
        (Some(standard), None) => Some(standard),
        (None, Some(synthetic)) => Some(synthetic),
        (None, None) => None,
    }
}

fn synthetic_event_boundary(buffer: &str) -> Option<(usize, usize)> {
    let mut cursor = 1;
    while cursor < buffer.len() {
        let candidate = next_event_start(&buffer[cursor..])?;
        let marker = cursor + candidate.0;
        if has_data_field(&buffer[..marker]) {
            return Some((marker, candidate.1));
        }
        cursor = marker + candidate.1;
    }
    None
}

fn synthetic_data_boundary(buffer: &str) -> Option<(usize, usize)> {
    let mut cursor = 1;
    while cursor < buffer.len() {
        let candidate = next_data_start(&buffer[cursor..])?;
        let marker = cursor + candidate.0;
        if frame_data_is_complete(&buffer[..marker]) && data_line_can_start_frame(&buffer[marker..])
        {
            return Some((marker, candidate.1));
        }
        cursor = marker + candidate.1;
    }
    None
}

fn earliest_boundary(boundaries: [Option<(usize, usize)>; 2]) -> Option<(usize, usize)> {
    boundaries
        .into_iter()
        .flatten()
        .min_by_key(|(offset, _)| *offset)
}

fn next_event_start(buffer: &str) -> Option<(usize, usize)> {
    match (
        buffer.find(SSE_CRLF_EVENT_START),
        buffer.find(SSE_LF_EVENT_START),
    ) {
        (Some(crlf), Some(lf)) if crlf <= lf => Some((crlf, "\r\n".len())),
        (Some(_), Some(lf)) => Some((lf, "\n".len())),
        (Some(crlf), None) => Some((crlf, "\r\n".len())),
        (None, Some(lf)) => Some((lf, "\n".len())),
        (None, None) => None,
    }
}

fn next_data_start(buffer: &str) -> Option<(usize, usize)> {
    match (
        buffer.find(SSE_CRLF_DATA_START),
        buffer.find(SSE_LF_DATA_START),
    ) {
        (Some(crlf), Some(lf)) if crlf <= lf => Some((crlf, "\r\n".len())),
        (Some(_), Some(lf)) => Some((lf, "\n".len())),
        (Some(crlf), None) => Some((crlf, "\r\n".len())),
        (None, Some(lf)) => Some((lf, "\n".len())),
        (None, None) => None,
    }
}

fn has_data_field(text: &str) -> bool {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    normalized.lines().any(|line| line.starts_with("data:"))
}

fn frame_data_is_complete(text: &str) -> bool {
    let data = frame_data(text);
    if data.trim() == SSE_DONE_DATA {
        return true;
    }
    serde_json::from_str::<Value>(&data).is_ok()
}

fn data_line_can_start_frame(text: &str) -> bool {
    let Some(line) = text
        .trim_start_matches(['\r', '\n'])
        .lines()
        .find(|line| line.starts_with("data:"))
    else {
        return false;
    };
    let value = field_value(line.strip_prefix("data:").unwrap_or_default()).trim_start();
    value.starts_with(JSON_OBJECT_START) || value == SSE_DONE_DATA
}

fn frame_data(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut data = String::new();
    for line in normalized.lines() {
        if !line.starts_with("data:") {
            continue;
        }
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(field_value(line.strip_prefix("data:").unwrap_or_default()));
    }
    data
}

fn parse_frame(key: SseKey, text: &str, complete: bool) -> ToolResult<Option<SseFrameEvent>> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut event = None;
    let mut data = String::new();
    for line in normalized.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = field_value(value);
        match field {
            SSE_FIELD_EVENT => event = Some(value.to_string()),
            SSE_FIELD_DATA => {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value);
            }
            _ => {}
        }
    }
    if event.is_none() && data.is_empty() {
        return Ok(None);
    }
    Ok(Some(SseFrameEvent {
        frame: SseFrame {
            pid: key.pid,
            stream_key: key.stream_key,
            direction: key.direction,
            event,
            data_bytes: data.len(),
            complete,
        },
        data,
    }))
}

fn field_value(value: &str) -> &str {
    value.strip_prefix(' ').unwrap_or(value)
}
