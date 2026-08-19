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

pub(super) struct Http1Analyzer {
    buffers: BTreeMap<StreamKey, StreamBuffer>,
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
        let append_outcome = if summary_only {
            buffer.append_summary_only(&segment.bytes, config.sse_max_buffer_bytes)?
        } else {
            buffer.append(&segment.bytes, config.sse_max_buffer_bytes)?
        };
        if append_outcome == StreamAppendOutcome::InvalidUtf8 {
            self.buffers.remove(&key);
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

            if !summary_only {
                if let Some(message) = buffer.take_chunked_sse_head(config)? {
                    if semantic_retention.http_message_summary_enabled() {
                        drafts.push(ApplicationEventDraft::complete(message.to_payload(
                            segment,
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
            }

            let Some(message) = buffer.take_message(config, summary_only)? else {
                break;
            };
            if semantic_retention.http_message_summary_enabled() {
                drafts.push(ApplicationEventDraft::complete(message.to_payload(
                    segment,
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
        if buffer.text.is_empty() && !buffer.expects_chunked_sse_body() {
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
        let mut payload = message.to_payload(segment, config, semantic_retention, consumed_by_llm);
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

struct StreamBuffer {
    text: String,
    state: StreamBufferState,
    utf8_tail: [u8; 3],
    utf8_tail_len: u8,
}

impl Default for StreamBuffer {
    fn default() -> Self {
        Self {
            text: String::new(),
            state: StreamBufferState::Http,
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

enum StreamBufferState {
    Http,
    ChunkedSse { pending_sse: String },
}

impl StreamBuffer {
    fn append(
        &mut self,
        bytes: &[u8],
        max_buffer_bytes: u64,
    ) -> Result<StreamAppendOutcome, String> {
        self.append_utf8(bytes, max_buffer_bytes)
    }

    fn append_summary_only(
        &mut self,
        bytes: &[u8],
        max_buffer_bytes: u64,
    ) -> Result<StreamAppendOutcome, String> {
        let started_empty = self.text.is_empty() && self.utf8_tail_len == 0;
        if started_empty && !starts_like_http_message_bytes(bytes) {
            return Ok(StreamAppendOutcome::Appended);
        }
        let header_input_len = http_header_input_len(&self.text, bytes);
        let outcome = self.append_utf8(&bytes[..header_input_len], max_buffer_bytes)?;
        if outcome == StreamAppendOutcome::InvalidUtf8 {
            return Ok(outcome);
        }
        if started_empty && parser::header_prefix_len(&self.text).is_none() {
            let first_line = self.text.lines().next().map(str::trim).unwrap_or_default();
            if !parser::starts_like_http_message(first_line) {
                self.clear();
                return Ok(StreamAppendOutcome::Appended);
            }
        }
        if let Some(prefix_len) = parser::header_prefix_len(&self.text) {
            self.text.truncate(prefix_len);
            self.utf8_tail_len = 0;
        }
        Ok(StreamAppendOutcome::Appended)
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

    fn clear(&mut self) {
        self.text.clear();
        self.utf8_tail_len = 0;
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

fn http_header_input_len(buffered: &str, bytes: &[u8]) -> usize {
    if parser::header_prefix_len(buffered).is_some() {
        return 0;
    }
    [b"\r\n\r\n".as_slice(), b"\n\n".as_slice()]
        .into_iter()
        .filter_map(|boundary| boundary_input_len(buffered.as_bytes(), bytes, boundary))
        .min()
        .unwrap_or(bytes.len())
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
    let line_end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(bytes.len());
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use model_core::ids::TraceId;
    use model_core::payload::{
        PayloadContentState, PayloadDirection, PayloadOperationCompletionState,
        PayloadRedactionState, PayloadSegmentId, PayloadSourceBoundary, PayloadTruncationState,
    };
    use model_core::process::ProcessIdentity;

    use super::*;

    #[test]
    fn incomplete_operation_emits_partial_http_head_without_body() {
        let bytes = b"POST /v1/messages HTTP/1.1\r\nHost: api.example.test\r\nContent-Length: 4096\r\n\r\n{\"model\":\"partial"
            .to_vec();
        let segment = incomplete_segment(bytes);
        let analyzer = Http1Analyzer::new(ApplicationProtocolConfig::default());

        let drafts = analyzer
            .analyze_incomplete_head(
                &segment,
                &ApplicationProtocolConfig::default(),
                &SemanticRetentionConfig::default(),
                false,
            )
            .unwrap();

        assert_eq!(drafts.len(), 1);
        let draft = &drafts[0];
        assert!(draft.metadata_partial);
        assert_eq!(draft.payload.protocol, "http/1.1");
        assert_eq!(draft.payload.operation, "request");
        assert_eq!(draft.payload.summary, "POST /v1/messages");
        assert!(draft.payload.body.is_none());
        assert_eq!(
            draft
                .payload
                .metadata
                .get("content_length")
                .map(String::as_str),
            Some("4096")
        );
        assert_eq!(
            draft
                .payload
                .metadata
                .get("payload.capture_incomplete")
                .map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn incomplete_operation_without_complete_head_emits_nothing() {
        let segment =
            incomplete_segment(b"POST /v1/messages HTTP/1.1\r\nContent-Length: 4096\r\n".to_vec());
        let analyzer = Http1Analyzer::new(ApplicationProtocolConfig::default());

        let drafts = analyzer
            .analyze_incomplete_head(
                &segment,
                &ApplicationProtocolConfig::default(),
                &SemanticRetentionConfig::default(),
                false,
            )
            .unwrap();

        assert!(drafts.is_empty());
    }

    #[test]
    fn incomplete_http2_preface_is_not_misclassified_as_http1() {
        let segment = incomplete_segment(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec());
        let analyzer = Http1Analyzer::new(ApplicationProtocolConfig::default());

        let drafts = analyzer
            .analyze_incomplete_head(
                &segment,
                &ApplicationProtocolConfig::default(),
                &SemanticRetentionConfig::default(),
                false,
            )
            .unwrap();

        assert!(drafts.is_empty());
    }

    fn incomplete_segment(bytes: Vec<u8>) -> PayloadSegment {
        let captured_size = bytes.len() as u64;
        PayloadSegment {
            segment_id: PayloadSegmentId::new(1),
            trace_id: TraceId::new(1),
            observed_at: UNIX_EPOCH + Duration::from_secs(1),
            process: ProcessIdentity::new(1),
            source_boundary: PayloadSourceBoundary::TlsUserSpace,
            content_state: PayloadContentState::Plaintext,
            direction: PayloadDirection::Outbound,
            stream_key: PayloadStreamKey::new("tls:1:42"),
            sequence: 1,
            original_size: captured_size + 1,
            captured_size,
            operation_id: 1,
            operation_offset: 0,
            operation_original_size: captured_size + 1,
            operation_captured_size: captured_size,
            operation_completion_state: PayloadOperationCompletionState::Partial,
            truncation: PayloadTruncationState::Truncated,
            redaction: PayloadRedactionState::NotRequired,
            library: "libssl.so".to_string(),
            symbol: "SSL_write".to_string(),
            protocol_hint: None,
            bytes,
        }
    }
}
