//! HTTP/1.x semantic analyzer over retained plaintext payload segments.

use std::collections::BTreeMap;

use config_core::daemon::ApplicationProtocolConfig;
use model_core::payload::PayloadSegment;

use super::ApplicationEventDraft;

#[path = "http1/parser.rs"]
mod parser;

pub(super) struct Http1Analyzer {
    config: ApplicationProtocolConfig,
    buffers: BTreeMap<String, StreamBuffer>,
}

impl Http1Analyzer {
    pub(super) fn new(config: ApplicationProtocolConfig) -> Self {
        Self {
            config,
            buffers: BTreeMap::new(),
        }
    }

    pub(super) fn analyze(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let text = match std::str::from_utf8(&segment.bytes) {
            Ok(text) => text,
            Err(_) => return Ok(Vec::new()),
        };
        let key = stream_key(segment);
        let buffer = self.buffers.entry(key.clone()).or_default();
        buffer.append(text, self.config.sse_max_buffer_bytes)?;
        if !buffer.starts_like_http_or_sse(self.config.sse_enabled) {
            self.buffers.remove(&key);
            return Ok(Vec::new());
        }

        let mut drafts = Vec::new();
        while let Some(message) = buffer.take_message(&self.config)? {
            drafts.push(ApplicationEventDraft {
                payload: message.to_payload(segment, &self.config),
            });
            if self.config.sse_enabled && message.is_sse() {
                for payload in message.sse_events(&self.config)? {
                    drafts.push(ApplicationEventDraft { payload });
                }
            }
        }
        if buffer.text.is_empty() {
            self.buffers.remove(&key);
        }
        Ok(drafts)
    }
}

#[derive(Default)]
struct StreamBuffer {
    text: String,
}

impl StreamBuffer {
    fn append(&mut self, text: &str, max_buffer_bytes: u64) -> Result<(), String> {
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
    ) -> Result<Option<parser::HttpMessage>, String> {
        parser::take_message(&mut self.text, config)
    }
}

fn stream_key(segment: &PayloadSegment) -> String {
    format!(
        "{}:{}:{:?}",
        segment.trace_id.get(),
        segment.stream_key,
        segment.direction
    )
}
