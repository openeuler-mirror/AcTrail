//! Incremental complete-event SSE framing.
//!
//! This component only receives decoded HTTP body bytes. HTTP/1 headers and
//! chunk framing must be removed first; HTTP/2 DATA must be demultiplexed by
//! stream ID first. A raw-SSE path must never take priority over either
//! transport decoder.
//!
//! Incomplete events produce no downstream event. The framer scans only the
//! unframed tail so a slow stream cannot cause repeated whole-body scans. Once
//! classifier budget is exhausted, later complete events remain eligible for
//! LLM recognition without repeatedly scanning the retained body. Retention
//! remains bounded by the configured response assembly limit. Once this
//! component owns a normalized response stream, the same bytes must not fall
//! through to a parallel legacy projection path.

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct CompleteSseEvent {
    pub(in crate::llm_pipeline) index: usize,
    pub(in crate::llm_pipeline) end_offset: usize,
    pub(in crate::llm_pipeline) event_type: Option<String>,
    pub(in crate::llm_pipeline) id: Option<String>,
    pub(in crate::llm_pipeline) data: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) enum SseFramingError {
    BodyShrank,
    InvalidUtf8,
}

/// Monotonic framer over a decoded response body that grows by prefix
/// extension. It retains offsets rather than a second copy of the body.
#[derive(Default)]
pub(in crate::llm_pipeline) struct IncrementalSseFramer {
    event_start: usize,
    scan_offset: usize,
    observed_body_len: usize,
    next_event_index: usize,
}

impl IncrementalSseFramer {
    pub(in crate::llm_pipeline) fn advance(
        &mut self,
        body: &[u8],
    ) -> Result<Vec<CompleteSseEvent>, SseFramingError> {
        if body.len() < self.observed_body_len {
            return Err(SseFramingError::BodyShrank);
        }
        self.observed_body_len = body.len();

        let mut events = Vec::new();
        let mut cursor = self.scan_offset.max(self.event_start);
        while let Some((delimiter_start, delimiter_len)) = next_event_delimiter(body, cursor) {
            let block = body
                .get(self.event_start..delimiter_start)
                .ok_or(SseFramingError::BodyShrank)?;
            let event_end = delimiter_start + delimiter_len;
            if let Some(event) = self.parse_event(block, event_end)? {
                events.push(event);
            }
            self.event_start = event_end;
            cursor = self.event_start;
        }
        self.scan_offset = body.len().saturating_sub(3).max(self.event_start);
        Ok(events)
    }

    pub(in crate::llm_pipeline) fn safe_consumed(&self) -> usize {
        self.event_start
    }

    fn parse_event(
        &mut self,
        block: &[u8],
        end_offset: usize,
    ) -> Result<Option<CompleteSseEvent>, SseFramingError> {
        let text = std::str::from_utf8(block).map_err(|_| SseFramingError::InvalidUtf8)?;
        let mut data_lines = Vec::new();
        let mut event_type = None;
        let mut id = None;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            let (name, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match name {
                "data" => data_lines.push(value),
                "event" => event_type = Some(value.to_string()),
                "id" => id = Some(value.to_string()),
                _ => {}
            }
        }
        if data_lines.is_empty() {
            return Ok(None);
        }
        let event = CompleteSseEvent {
            index: self.next_event_index,
            end_offset,
            event_type,
            id,
            data: data_lines.join("\n"),
        };
        self.next_event_index += 1;
        Ok(Some(event))
    }
}

fn next_event_delimiter(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut cursor = from;
    while cursor < bytes.len() {
        let remaining = &bytes[cursor..];
        if remaining.starts_with(b"\r\n\r\n") {
            return Some((cursor, 4));
        }
        if remaining.starts_with(b"\n\n") {
            return Some((cursor, 2));
        }
        cursor += 1;
    }
    None
}
