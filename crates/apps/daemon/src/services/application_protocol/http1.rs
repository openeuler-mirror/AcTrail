//! HTTP/1.x semantic analyzer over retained plaintext payload segments.

use std::collections::BTreeMap;

use config_core::daemon::ApplicationProtocolConfig;
use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment, PayloadStreamKey};
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
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let text = match std::str::from_utf8(&segment.bytes) {
            Ok(text) => text,
            Err(_) => return Ok(Vec::new()),
        };
        let key = stream_key(segment);
        let buffer = self.buffers.entry(key.clone()).or_default();
        if summary_only {
            buffer.append_summary_only(text, config.sse_max_buffer_bytes)?;
        } else {
            buffer.append(text, config.sse_max_buffer_bytes)?;
        }
        if !buffer.starts_like_http_or_sse(config.sse_enabled) {
            self.buffers.remove(&key);
            return Ok(Vec::new());
        }

        let mut drafts = Vec::new();
        while let Some(message) = buffer.take_message(config, summary_only)? {
            drafts.push(ApplicationEventDraft {
                payload: message.to_payload(segment, config),
            });
            if config.sse_enabled && message.is_sse() {
                for payload in message.sse_events(config)? {
                    drafts.push(ApplicationEventDraft { payload });
                }
            }
        }
        if buffer.text.is_empty() {
            self.buffers.remove(&key);
        }
        Ok(drafts)
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.buffers.retain(|key, _| key.trace_id != trace_id);
    }

    pub(super) fn forget_stream(&mut self, segment: &PayloadSegment) {
        self.buffers.retain(|key, _| {
            key.trace_id != segment.trace_id
                || key.process != segment.process
                || key.stream_key != segment.stream_key
        });
    }

    #[cfg(test)]
    pub(super) fn buffered_stream_count(&self) -> usize {
        self.buffers.len()
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

#[derive(Default)]
struct StreamBuffer {
    text: String,
}

impl StreamBuffer {
    fn append(&mut self, text: &str, max_buffer_bytes: u64) -> Result<(), String> {
        self.append_checked(text, max_buffer_bytes)
    }

    fn append_summary_only(&mut self, text: &str, max_buffer_bytes: u64) -> Result<(), String> {
        if self.text.is_empty() && parser::header_prefix_len(text).is_none() {
            let first_line = text.lines().next().map(str::trim).unwrap_or_default();
            if !parser::starts_like_http_message(first_line) {
                return Ok(());
            }
        }
        let prefix = parser::header_prefix_len(text)
            .and_then(|prefix_len| text.get(..prefix_len))
            .unwrap_or(text);
        self.append_checked(prefix, max_buffer_bytes)?;
        if let Some(prefix_len) = parser::header_prefix_len(&self.text) {
            self.text.truncate(prefix_len);
        }
        Ok(())
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

    fn take_message(
        &mut self,
        config: &ApplicationProtocolConfig,
        summary_only: bool,
    ) -> Result<Option<parser::HttpMessage>, String> {
        parser::take_message(&mut self.text, config, summary_only)
    }
}

fn stream_key(segment: &PayloadSegment) -> StreamKey {
    StreamKey {
        trace_id: segment.trace_id,
        process: segment.process.clone(),
        stream_key: segment.stream_key.clone(),
        direction: StreamDirectionKey::from(segment.direction),
    }
}
