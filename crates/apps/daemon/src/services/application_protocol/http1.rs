//! HTTP/1.x semantic analyzer over retained plaintext payload segments.

use std::collections::BTreeMap;

use config_core::daemon::{ApplicationProtocolConfig, SemanticRetentionConfig};
use model_core::event::ApplicationPayload;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadDirection, PayloadSegment, PayloadStreamIdentity, PayloadStreamKey,
};
use model_core::process::ProcessIdentity;

use super::ApplicationEventDraft;

#[path = "http1/parser.rs"]
mod parser;
#[path = "http1/summary_chunked.rs"]
mod summary_chunked;

pub(super) struct Http1Analyzer {
    buffers: BTreeMap<StreamKey, StreamBuffer>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IncompleteHttp1Message {
    Request {
        sequence: u64,
        header_projected: bool,
    },
    Response {
        sequence: u64,
        header_projected: bool,
    },
}

impl Http1Analyzer {
    pub(super) fn new(_config: ApplicationProtocolConfig) -> Self {
        Self {
            buffers: BTreeMap::new(),
        }
    }

    pub(super) fn analyze_with_config(
        &mut self,
        segment: &PayloadSegment,
        config: &ApplicationProtocolConfig,
        semantic_retention: &SemanticRetentionConfig,
        consumed_by_llm: bool,
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let key = stream_key(segment);
        let buffer = self.buffers.entry(key.clone()).or_default();
        if buffer.is_opaque() {
            return Ok(Vec::new());
        }
        if summary_only {
            let (append_outcome, messages) = buffer.take_summary_messages(
                &segment.bytes,
                segment.sequence,
                &config,
                config.sse_max_buffer_bytes,
            )?;
            if append_outcome == StreamAppendOutcome::InvalidUtf8 {
                self.buffers.remove(&key);
                return Ok(Vec::new());
            }
            let drafts = messages
                .into_iter()
                .filter(|_| semantic_retention.http_message_summary_enabled())
                .map(|(message, sequence)| {
                    ApplicationEventDraft::complete(message.to_payload(
                        segment,
                        sequence,
                        &config,
                        semantic_retention,
                        consumed_by_llm,
                    ))
                })
                .collect();
            if buffer.text.is_empty() && !buffer.summary_body_in_progress() && !buffer.is_opaque() {
                self.buffers.remove(&key);
            }
            return Ok(drafts);
        }
        let append_outcome = buffer.append(&segment.bytes, config.sse_max_buffer_bytes)?;
        if append_outcome == StreamAppendOutcome::InvalidUtf8 {
            self.buffers.remove(&key);
            return Ok(Vec::new());
        }
        if buffer.summary_body_in_progress() {
            return Ok(Vec::new());
        }
        if !buffer.expects_chunked_sse_body() && !buffer.starts_like_http_or_sse(config.sse_enabled)
        {
            self.buffers.remove(&key);
            return Ok(Vec::new());
        }

        let mut drafts = Vec::new();
        loop {
            if buffer.expects_chunked_sse_body() {
                let drain = buffer.take_chunked_sse_events(config)?;
                for payload in drain.payloads {
                    drafts.push(ApplicationEventDraft::complete(payload));
                }
                if drain.done {
                    buffer.finish_chunked_sse_body();
                    if !buffer.text.is_empty() {
                        continue;
                    }
                }
                break;
            }

            if let Some(message) = buffer.take_chunked_sse_head(config)? {
                if semantic_retention.http_message_summary_enabled() {
                    drafts.push(ApplicationEventDraft::complete(message.to_payload(
                        segment,
                        segment.sequence,
                        config,
                        semantic_retention,
                        consumed_by_llm,
                    )));
                }
                buffer.start_chunked_sse_body();
                continue;
            }
            for payload in buffer.take_streaming_sse_events(config)? {
                drafts.push(ApplicationEventDraft::complete(payload));
            }

            let Some(message) = buffer.take_message(config, false)? else {
                break;
            };
            if semantic_retention.http_message_summary_enabled() {
                drafts.push(ApplicationEventDraft::complete(message.to_payload(
                    segment,
                    segment.sequence,
                    config,
                    semantic_retention,
                    consumed_by_llm,
                )));
            }
            if config.sse_enabled && message.is_sse() {
                for payload in message.sse_events(config)? {
                    drafts.push(ApplicationEventDraft::complete(payload));
                }
            }
        }
        if buffer.text.is_empty()
            && !buffer.expects_chunked_sse_body()
            && !buffer.summary_body_in_progress()
            && !buffer.is_opaque()
        {
            self.buffers.remove(&key);
        }
        Ok(drafts)
    }

    pub(super) fn analyze_incomplete_head(
        &self,
        segment: &PayloadSegment,
        config: &ApplicationProtocolConfig,
        semantic_retention: &SemanticRetentionConfig,
        consumed_by_llm: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let Some(message) = parser::parse_complete_message_head(&segment.bytes)? else {
            return Ok(Vec::new());
        };
        if !semantic_retention.http_message_summary_enabled() {
            return Ok(Vec::new());
        }
        let mut payload = message.to_payload(
            segment,
            segment.sequence,
            config,
            semantic_retention,
            consumed_by_llm,
        );
        payload.metadata.extend([
            ("payload.capture_incomplete".to_string(), "true".to_string()),
            (
                "payload.operation_completion_state".to_string(),
                segment.operation_completion_state.as_str().to_string(),
            ),
            (
                "payload.operation_original_size".to_string(),
                segment.operation_original_size.to_string(),
            ),
            (
                "payload.operation_captured_size".to_string(),
                segment.operation_captured_size.to_string(),
            ),
            (
                "payload.truncation".to_string(),
                format!("{:?}", segment.truncation).to_ascii_lowercase(),
            ),
        ]);
        Ok(vec![ApplicationEventDraft::partial(payload)])
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.buffers.retain(|key, _| key.trace_id != trace_id);
    }

    pub(super) fn forget_stream(&mut self, identity: &PayloadStreamIdentity) {
        self.buffers.retain(|key, _| {
            key.trace_id != identity.trace_id
                || key.process != identity.process
                || key.stream_key != identity.stream_key
        });
    }

    pub(super) fn forget_segment_direction(&mut self, segment: &PayloadSegment) {
        self.buffers.remove(&stream_key(segment));
    }

    pub(super) fn incomplete_segment_message(
        &self,
        segment: &PayloadSegment,
        _max_buffer_bytes: u64,
    ) -> Option<IncompleteHttp1Message> {
        if segment.operation_original_size <= segment.operation_captured_size {
            return None;
        }
        let buffer = self.buffers.get(&stream_key(segment))?;
        buffer.incomplete_segment_message(segment)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StreamKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: PayloadStreamKey,
    direction: StreamDirectionKey,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum StreamDirectionKey {
    Outbound,
    Inbound,
}

impl From<PayloadDirection> for StreamDirectionKey {
    fn from(value: PayloadDirection) -> Self {
        match value {
            PayloadDirection::Outbound => Self::Outbound,
            PayloadDirection::Inbound => Self::Inbound,
        }
    }
}

#[derive(Clone)]
struct StreamBuffer {
    text: String,
    state: StreamBufferState,
    summary_body: Option<SummaryBodyProgress>,
    summary_chunked: Option<SummaryChunkedProgress>,
    summary_header_sequence: Option<u64>,
    utf8_tail: [u8; 3],
    utf8_tail_len: u8,
}

impl Default for StreamBuffer {
    fn default() -> Self {
        Self {
            text: String::new(),
            state: StreamBufferState::Http,
            summary_body: None,
            summary_chunked: None,
            summary_header_sequence: None,
            utf8_tail: [0; 3],
            utf8_tail_len: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamAppendOutcome {
    Appended,
    InvalidUtf8,
}

#[derive(Clone)]
enum StreamBufferState {
    Http,
    ChunkedSse { pending_sse: String },
    Opaque,
}

#[derive(Clone)]
struct SummaryBodyProgress {
    remaining: u64,
    request: bool,
    sequence: u64,
}

#[derive(Clone)]
struct SummaryChunkedProgress {
    state: SummaryChunkedState,
}

#[derive(Clone)]
enum SummaryChunkedState {
    Size {
        value: u64,
        saw_digit: bool,
        in_extension: bool,
        saw_cr: bool,
        line_bytes: u64,
    },
    Data(u64),
    DataTerminator(u8),
    Trailers {
        line_nonempty: bool,
        saw_cr: bool,
        line_bytes: u64,
    },
}

impl Default for SummaryChunkedProgress {
    fn default() -> Self {
        Self {
            state: SummaryChunkedState::Size {
                value: 0,
                saw_digit: false,
                in_extension: false,
                saw_cr: false,
                line_bytes: 0,
            },
        }
    }
}

impl StreamBuffer {
    fn incomplete_segment_message(
        &self,
        segment: &PayloadSegment,
    ) -> Option<IncompleteHttp1Message> {
        if let Some(progress) = &self.summary_body {
            let missing_bytes = segment
                .operation_original_size
                .saturating_sub(segment.operation_captured_size);
            let within_message = segment
                .captured_size
                .checked_add(missing_bytes)
                .is_some_and(|remaining_operation| remaining_operation <= progress.remaining);
            let message = if progress.request {
                IncompleteHttp1Message::Request {
                    sequence: progress.sequence,
                    header_projected: true,
                }
            } else {
                IncompleteHttp1Message::Response {
                    sequence: progress.sequence,
                    header_projected: true,
                }
            };
            return (within_message
                && matches!(
                    (segment.direction, message),
                    (
                        PayloadDirection::Outbound,
                        IncompleteHttp1Message::Request { .. }
                    ) | (
                        PayloadDirection::Inbound,
                        IncompleteHttp1Message::Response { .. }
                    )
                ))
            .then_some(message);
        }
        None
    }

    fn append(
        &mut self,
        bytes: &[u8],
        max_buffer_bytes: u64,
    ) -> Result<StreamAppendOutcome, String> {
        if matches!(self.state, StreamBufferState::Opaque) {
            return Ok(StreamAppendOutcome::Appended);
        }
        self.append_utf8(bytes, max_buffer_bytes)
    }

    fn take_summary_messages(
        &mut self,
        mut bytes: &[u8],
        sequence: u64,
        config: &ApplicationProtocolConfig,
        max_buffer_bytes: u64,
    ) -> Result<(StreamAppendOutcome, Vec<(parser::HttpMessage, u64)>), String> {
        let mut messages = Vec::new();
        if matches!(self.state, StreamBufferState::Opaque) {
            return Ok((StreamAppendOutcome::Appended, messages));
        }
        loop {
            if let Some(progress) = self.summary_chunked.as_mut() {
                let (consumed, done) = match progress.consume(bytes, max_buffer_bytes) {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        self.summary_chunked = None;
                        self.state = StreamBufferState::Opaque;
                        return Ok((StreamAppendOutcome::Appended, messages));
                    }
                };
                bytes = &bytes[consumed..];
                if !done {
                    return Ok((StreamAppendOutcome::Appended, messages));
                }
                self.summary_chunked = None;
                if bytes.is_empty() {
                    return Ok((StreamAppendOutcome::Appended, messages));
                }
            }
            if let Some(progress) = self.summary_body.as_mut() {
                let take = usize::try_from(progress.remaining)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                progress.remaining = progress.remaining.saturating_sub(take as u64);
                bytes = &bytes[take..];
                if progress.remaining != 0 {
                    return Ok((StreamAppendOutcome::Appended, messages));
                }
                self.summary_body = None;
                if bytes.is_empty() {
                    return Ok((StreamAppendOutcome::Appended, messages));
                }
            }
            let started_empty = self.text.is_empty() && self.utf8_tail_len == 0;
            if started_empty {
                if !starts_like_http_message_bytes(bytes) {
                    return Ok((StreamAppendOutcome::Appended, messages));
                }
                self.summary_header_sequence = Some(sequence);
            }
            let header_end = http_header_input_len(&self.text, bytes);
            let header_input_len = header_end.unwrap_or(bytes.len());
            let tail = &bytes[header_input_len..];
            let outcome = self.append_utf8(&bytes[..header_input_len], max_buffer_bytes)?;
            if outcome == StreamAppendOutcome::InvalidUtf8 {
                return Ok((outcome, messages));
            }
            if header_end.is_none() {
                return Ok((StreamAppendOutcome::Appended, messages));
            }
            self.utf8_tail_len = 0;
            let Some(message) = self.take_message(config, true)? else {
                return Ok((StreamAppendOutcome::Appended, messages));
            };
            let framing = message.summary_framing();
            let request = message.is_request();
            let message_sequence = self.summary_header_sequence.take().unwrap_or(sequence);
            messages.push((message, message_sequence));
            let content_length = match framing {
                parser::SummaryFraming::NoBody => 0,
                parser::SummaryFraming::Fixed(content_length) => content_length,
                parser::SummaryFraming::Chunked => {
                    self.summary_chunked = Some(SummaryChunkedProgress::default());
                    bytes = tail;
                    continue;
                }
                parser::SummaryFraming::Unsupported => {
                    self.state = StreamBufferState::Opaque;
                    return Ok((StreamAppendOutcome::Appended, messages));
                }
            };
            let body_bytes = content_length.min(tail.len());
            bytes = &tail[body_bytes..];
            let remaining = content_length.saturating_sub(body_bytes);
            if remaining != 0 {
                self.summary_body = Some(SummaryBodyProgress {
                    remaining: remaining as u64,
                    request,
                    sequence: message_sequence,
                });
                return Ok((StreamAppendOutcome::Appended, messages));
            }
            if bytes.is_empty() {
                return Ok((StreamAppendOutcome::Appended, messages));
            }
        }
    }

    fn summary_body_in_progress(&self) -> bool {
        self.summary_body.is_some() || self.summary_chunked.is_some()
    }

    fn is_opaque(&self) -> bool {
        matches!(self.state, StreamBufferState::Opaque)
    }

    fn append_utf8(
        &mut self,
        mut bytes: &[u8],
        max_buffer_bytes: u64,
    ) -> Result<StreamAppendOutcome, String> {
        if self.utf8_tail_len != 0 {
            let tail_len = usize::from(self.utf8_tail_len);
            let Some(width) = utf8_sequence_width(self.utf8_tail[0]) else {
                return Ok(StreamAppendOutcome::InvalidUtf8);
            };
            let take = width.saturating_sub(tail_len).min(bytes.len());
            let mut candidate = [0_u8; 4];
            candidate[..tail_len].copy_from_slice(&self.utf8_tail[..tail_len]);
            candidate[tail_len..tail_len + take].copy_from_slice(&bytes[..take]);
            let candidate_len = tail_len + take;
            match std::str::from_utf8(&candidate[..candidate_len]) {
                Ok(text) => {
                    self.append_checked(text, max_buffer_bytes)?;
                    self.utf8_tail_len = 0;
                    bytes = &bytes[take..];
                }
                Err(error) if error.error_len().is_none() && candidate_len <= 3 => {
                    self.utf8_tail[..candidate_len].copy_from_slice(&candidate[..candidate_len]);
                    self.utf8_tail_len = candidate_len as u8;
                    return Ok(StreamAppendOutcome::Appended);
                }
                Err(_) => return Ok(StreamAppendOutcome::InvalidUtf8),
            }
        }

        match std::str::from_utf8(bytes) {
            Ok(text) => {
                self.append_checked(text, max_buffer_bytes)?;
                Ok(StreamAppendOutcome::Appended)
            }
            Err(error) if error.error_len().is_none() => {
                let valid_up_to = error.valid_up_to();
                let Ok(valid) = std::str::from_utf8(&bytes[..valid_up_to]) else {
                    return Ok(StreamAppendOutcome::InvalidUtf8);
                };
                self.append_checked(valid, max_buffer_bytes)?;
                let tail = &bytes[valid_up_to..];
                if tail.len() > self.utf8_tail.len() {
                    return Ok(StreamAppendOutcome::InvalidUtf8);
                }
                self.utf8_tail[..tail.len()].copy_from_slice(tail);
                self.utf8_tail_len = tail.len() as u8;
                Ok(StreamAppendOutcome::Appended)
            }
            Err(_) => Ok(StreamAppendOutcome::InvalidUtf8),
        }
    }

    fn append_checked(&mut self, text: &str, max_buffer_bytes: u64) -> Result<(), String> {
        let next_len = self
            .text
            .len()
            .checked_add(text.len())
            .ok_or_else(|| "application HTTP/1.x buffer size overflow".to_string())?;
        if u64::try_from(next_len).map_err(|error| error.to_string())? > max_buffer_bytes {
            return Err(format!(
                "application HTTP/1.x buffer would exceed configured maximum {max_buffer_bytes} bytes"
            ));
        }
        self.text.push_str(text);
        Ok(())
    }

    fn starts_like_http_or_sse(&self, sse_enabled: bool) -> bool {
        parser::starts_like_http_or_sse(&self.text, sse_enabled)
    }

    fn expects_chunked_sse_body(&self) -> bool {
        matches!(self.state, StreamBufferState::ChunkedSse { .. })
    }

    fn start_chunked_sse_body(&mut self) {
        self.state = StreamBufferState::ChunkedSse {
            pending_sse: String::new(),
        };
    }

    fn finish_chunked_sse_body(&mut self) {
        self.state = StreamBufferState::Http;
    }

    fn take_message(
        &mut self,
        config: &ApplicationProtocolConfig,
        summary_only: bool,
    ) -> Result<Option<parser::HttpMessage>, String> {
        parser::take_message(&mut self.text, config, summary_only)
    }

    fn take_chunked_sse_head(
        &mut self,
        config: &ApplicationProtocolConfig,
    ) -> Result<Option<parser::HttpMessage>, String> {
        parser::take_chunked_sse_head(&mut self.text, config)
    }

    fn take_streaming_sse_events(
        &mut self,
        config: &ApplicationProtocolConfig,
    ) -> Result<Vec<ApplicationPayload>, String> {
        parser::take_streaming_sse_events(&mut self.text, config)
    }

    fn take_chunked_sse_events(
        &mut self,
        config: &ApplicationProtocolConfig,
    ) -> Result<parser::ChunkedSseDrain, String> {
        match &mut self.state {
            StreamBufferState::Http => Ok(parser::ChunkedSseDrain {
                payloads: Vec::new(),
                done: false,
            }),
            StreamBufferState::ChunkedSse { pending_sse } => {
                parser::take_chunked_sse_events(&mut self.text, pending_sse, config)
            }
            StreamBufferState::Opaque => Ok(parser::ChunkedSseDrain {
                payloads: Vec::new(),
                done: false,
            }),
        }
    }
}

fn utf8_sequence_width(first: u8) -> Option<usize> {
    match first {
        0xC2..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF4 => Some(4),
        _ => None,
    }
}

fn http_header_input_len(buffered: &str, bytes: &[u8]) -> Option<usize> {
    [b"\r\n\r\n".as_slice(), b"\n\n".as_slice()]
        .into_iter()
        .filter_map(|boundary| boundary_input_len(buffered.as_bytes(), bytes, boundary))
        .min()
}

fn boundary_input_len(buffered: &[u8], bytes: &[u8], boundary: &[u8]) -> Option<usize> {
    for split in (1..boundary.len()).rev() {
        if buffered.ends_with(&boundary[..split]) && bytes.starts_with(&boundary[split..]) {
            return Some(boundary.len() - split);
        }
    }
    bytes
        .windows(boundary.len())
        .position(|window| window == boundary)
        .map(|position| position + boundary.len())
}

fn starts_like_http_message_bytes(bytes: &[u8]) -> bool {
    let first = bytes.first().copied().unwrap_or_default();
    if first != b'H' && !first.is_ascii_uppercase() {
        return false;
    }
    let line_end = bytes.iter().position(|byte| *byte == b'\n');
    let Some(line_end) = line_end else {
        return bytes
            .iter()
            .all(|byte| byte.is_ascii_graphic() || *byte == b' ' || *byte == b'\r');
    };
    std::str::from_utf8(&bytes[..line_end])
        .ok()
        .map(str::trim)
        .is_some_and(parser::starts_like_http_message)
}

fn stream_key(segment: &PayloadSegment) -> StreamKey {
    StreamKey {
        trace_id: segment.trace_id,
        process: segment.process.clone(),
        stream_key: segment.stream_key.clone(),
        direction: StreamDirectionKey::from(segment.direction),
    }
}
