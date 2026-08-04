use std::collections::VecDeque;
use std::time::SystemTime;

use model_core::payload::{PayloadSegment, PayloadTruncationState};
use semantic_action::{SemanticEvidence, SemanticEvidenceKind, evidence_roles};
use serde_json::Value;

use super::model::McpJsonRpcMessage;

#[derive(Clone, Debug)]
pub(super) struct McpJsonRpcFramer {
    max_buffer_bytes: usize,
    buffer: Vec<u8>,
    base_offset: usize,
    segments: VecDeque<McpSegmentRange>,
    pending: McpPendingFrame,
}

#[derive(Clone, Debug)]
struct McpSegmentRange {
    start: usize,
    end: usize,
    segment_id: Option<u64>,
    observed_at: SystemTime,
}

#[derive(Clone, Copy, Debug, Default)]
enum McpPendingFrame {
    #[default]
    Unknown,
    JsonLine {
        scanned_len: usize,
        prefix_validated: bool,
    },
    ContentLengthHeader {
        scanned_len: usize,
    },
    ContentLengthBody {
        body_start: usize,
        body_end: usize,
    },
}

#[derive(Clone, Copy, Debug)]
enum McpCompleteFrame {
    JsonLine {
        encoded_len: usize,
    },
    ContentLength {
        encoded_len: usize,
        body_start: usize,
        body_end: usize,
    },
}

pub(super) struct McpConfirmedFramingObservation {
    pub(super) messages: Vec<McpJsonRpcMessage>,
    pub(super) discarded_reason: Option<&'static str>,
}

pub(super) struct McpCandidateFramingObservation {
    pub(super) messages: Vec<McpJsonRpcMessage>,
    pub(super) rejection: Option<&'static str>,
}

impl McpJsonRpcFramer {
    pub(super) fn new(max_buffer_bytes: usize) -> Self {
        Self {
            max_buffer_bytes,
            buffer: Vec::new(),
            base_offset: 0,
            segments: VecDeque::new(),
            pending: McpPendingFrame::Unknown,
        }
    }

    pub(super) fn set_max_buffer_bytes(&mut self, max_buffer_bytes: usize) {
        self.max_buffer_bytes = max_buffer_bytes;
    }

    pub(super) fn observe_candidate(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> McpCandidateFramingObservation {
        if segment.truncation == PayloadTruncationState::Truncated {
            self.discard_buffer();
            return McpCandidateFramingObservation {
                messages: Vec::new(),
                rejection: Some("candidate_truncated"),
            };
        }
        if let Err(rejection) = self.append_segment(segment, retain_evidence) {
            return McpCandidateFramingObservation {
                messages: Vec::new(),
                rejection: Some(rejection),
            };
        }
        let (messages, rejection) = self.parse_complete_messages();
        McpCandidateFramingObservation {
            messages,
            rejection,
        }
    }

    pub(super) fn observe_confirmed(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> McpConfirmedFramingObservation {
        if segment.truncation == PayloadTruncationState::Truncated {
            self.discard_buffer();
            return McpConfirmedFramingObservation {
                messages: Vec::new(),
                discarded_reason: Some("confirmed_truncated"),
            };
        }
        if let Err(reason) = self.append_segment(segment, retain_evidence) {
            self.discard_buffer();
            return McpConfirmedFramingObservation {
                messages: Vec::new(),
                discarded_reason: Some(reason),
            };
        }
        let (messages, rejection) = self.parse_complete_messages();
        McpConfirmedFramingObservation {
            messages,
            discarded_reason: rejection,
        }
    }

    fn append_segment(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> Result<(), &'static str> {
        if segment.bytes.is_empty() {
            return Ok(());
        }
        let next_len = self
            .buffer
            .len()
            .checked_add(segment.bytes.len())
            .ok_or("framing_size_overflow")?;
        if next_len > self.max_buffer_bytes {
            return Err("framing_size_limit");
        }
        let start = self
            .base_offset
            .checked_add(self.buffer.len())
            .ok_or("framing_offset_overflow")?;
        self.buffer.extend_from_slice(&segment.bytes);
        let end = self
            .base_offset
            .checked_add(self.buffer.len())
            .ok_or("framing_offset_overflow")?;
        self.segments.push_back(McpSegmentRange {
            start,
            end,
            segment_id: retain_evidence.then(|| segment.segment_id.get()),
            observed_at: segment.observed_at,
        });
        Ok(())
    }

    fn parse_complete_messages(&mut self) -> (Vec<McpJsonRpcMessage>, Option<&'static str>) {
        let mut messages = Vec::new();
        let mut parsed_len = 0usize;
        let result = self.parse_complete_messages_inner(&mut messages, &mut parsed_len);
        if parsed_len > 0 {
            self.evict_encoded_prefix(parsed_len);
        }
        if result.is_err() {
            self.discard_buffer();
        }
        (messages, result.err())
    }

    fn parse_complete_messages_inner(
        &mut self,
        messages: &mut Vec<McpJsonRpcMessage>,
        parsed_len: &mut usize,
    ) -> Result<(), &'static str> {
        loop {
            let empty_line_len = Self::leading_empty_line_len(&self.buffer[*parsed_len..]);
            if empty_line_len > 0 {
                *parsed_len += empty_line_len;
                self.pending = McpPendingFrame::Unknown;
                continue;
            }
            if *parsed_len == self.buffer.len() {
                break;
            }
            let Some(frame) = self.next_frame(*parsed_len)? else {
                break;
            };
            let remaining = &self.buffer[*parsed_len..];
            let (encoded_len, value) = match frame {
                McpCompleteFrame::JsonLine { encoded_len } => {
                    let line = std::str::from_utf8(&remaining[..encoded_len])
                        .map_err(|_| "invalid_utf8_framing")?;
                    if line.trim().is_empty() {
                        *parsed_len += encoded_len;
                        continue;
                    }
                    (
                        encoded_len,
                        serde_json::from_str::<Value>(line.trim()).map_err(|_| "invalid_json")?,
                    )
                }
                McpCompleteFrame::ContentLength {
                    encoded_len,
                    body_start,
                    body_end,
                } => (
                    encoded_len,
                    serde_json::from_slice::<Value>(&remaining[body_start..body_end])
                        .map_err(|_| "invalid_json")?,
                ),
            };
            let start = self
                .base_offset
                .checked_add(*parsed_len)
                .ok_or("framing_offset_overflow")?;
            let end = start
                .checked_add(encoded_len)
                .ok_or("framing_offset_overflow")?;
            let (evidence, observed_at) = self.message_context(start, end);
            messages.extend(McpJsonRpcMessage::split_complete_value(
                value,
                observed_at,
                evidence,
            ));
            *parsed_len += encoded_len;
        }
        Ok(())
    }

    fn next_frame(&mut self, parsed_len: usize) -> Result<Option<McpCompleteFrame>, &'static str> {
        let buffer = &self.buffer[parsed_len..];
        let pending = &mut self.pending;
        if let McpPendingFrame::ContentLengthBody {
            body_start,
            body_end,
        } = *pending
        {
            if buffer.len() < body_end {
                return Ok(None);
            }
            *pending = McpPendingFrame::Unknown;
            return Ok(Some(McpCompleteFrame::ContentLength {
                encoded_len: body_end,
                body_start,
                body_end,
            }));
        }

        const CONTENT_LENGTH_PREFIX: &[u8] = b"Content-Length:";
        let content_length = buffer.len() >= CONTENT_LENGTH_PREFIX.len()
            && buffer[..CONTENT_LENGTH_PREFIX.len()].eq_ignore_ascii_case(CONTENT_LENGTH_PREFIX);
        let partial_content_length = buffer.len() < CONTENT_LENGTH_PREFIX.len()
            && buffer
                .iter()
                .zip(CONTENT_LENGTH_PREFIX)
                .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected));
        if content_length || partial_content_length {
            return Self::next_content_length_frame(pending, self.max_buffer_bytes, buffer);
        }
        Self::next_json_line_frame(pending, buffer)
    }

    fn next_json_line_frame(
        pending: &mut McpPendingFrame,
        buffer: &[u8],
    ) -> Result<Option<McpCompleteFrame>, &'static str> {
        let (scanned_len, prefix_validated) = match *pending {
            McpPendingFrame::JsonLine {
                scanned_len,
                prefix_validated,
            } => (scanned_len.min(buffer.len()), prefix_validated),
            McpPendingFrame::Unknown => (0, false),
            _ => return Err("invalid_framing_state"),
        };
        let Some(encoded_len) = buffer[scanned_len..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| scanned_len + position + 1)
        else {
            let mut prefix_validated = prefix_validated;
            if !prefix_validated
                && let Some(first) = buffer[scanned_len..]
                    .iter()
                    .find(|byte| !byte.is_ascii_whitespace())
            {
                if !matches!(*first, b'{' | b'[') {
                    return Err("invalid_framing");
                }
                prefix_validated = true;
            }
            *pending = McpPendingFrame::JsonLine {
                scanned_len: buffer.len(),
                prefix_validated,
            };
            return Ok(None);
        };
        *pending = McpPendingFrame::Unknown;
        Ok(Some(McpCompleteFrame::JsonLine { encoded_len }))
    }

    fn next_content_length_frame(
        pending: &mut McpPendingFrame,
        max_buffer_bytes: usize,
        buffer: &[u8],
    ) -> Result<Option<McpCompleteFrame>, &'static str> {
        let scanned_len = match *pending {
            McpPendingFrame::ContentLengthHeader { scanned_len } => scanned_len.min(buffer.len()),
            McpPendingFrame::Unknown => 0,
            _ => return Err("invalid_framing_state"),
        };
        let search_from = scanned_len.saturating_sub(3);
        let delimiter = Self::find_subslice(&buffer[search_from..], b"\r\n\r\n")
            .map(|position| search_from + position);
        if delimiter.is_none()
            && Self::find_subslice(&buffer[scanned_len.saturating_sub(1)..], b"\n\n").is_some()
        {
            return Err("invalid_content_length_delimiter");
        }
        let Some(header_end) = delimiter else {
            *pending = McpPendingFrame::ContentLengthHeader {
                scanned_len: buffer.len(),
            };
            return Ok(None);
        };
        let headers = std::str::from_utf8(&buffer[..header_end])
            .map_err(|_| "invalid_content_length_header")?;
        let mut content_length = None;
        for line in headers.split("\r\n") {
            let Some((name, value)) = line.split_once(':') else {
                return Err("invalid_content_length_header");
            };
            if !name.eq_ignore_ascii_case("Content-Length") {
                continue;
            }
            if content_length.is_some() {
                return Err("duplicate_content_length");
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid_content_length")?,
            );
        }
        let content_length = content_length.ok_or("missing_content_length")?;
        let body_start = header_end.checked_add(4).ok_or("framing_size_overflow")?;
        let body_end = body_start
            .checked_add(content_length)
            .ok_or("framing_size_overflow")?;
        if body_end > max_buffer_bytes {
            return Err("framing_size_limit");
        }
        if buffer.len() < body_end {
            *pending = McpPendingFrame::ContentLengthBody {
                body_start,
                body_end,
            };
            return Ok(None);
        }
        *pending = McpPendingFrame::Unknown;
        Ok(Some(McpCompleteFrame::ContentLength {
            encoded_len: body_end,
            body_start,
            body_end,
        }))
    }

    fn message_context(&self, start: usize, end: usize) -> (Vec<SemanticEvidence>, SystemTime) {
        let mut observed_at = SystemTime::UNIX_EPOCH;
        let first = self
            .segments
            .partition_point(|segment| segment.end <= start);
        let evidence = self
            .segments
            .iter()
            .skip(first)
            .take_while(|segment| segment.start < end)
            .filter_map(|segment| {
                observed_at = observed_at.max(segment.observed_at);
                segment.segment_id.map(|id| SemanticEvidence {
                    kind: SemanticEvidenceKind::PayloadSegment,
                    id,
                    role: evidence_roles::mcp::TOOL_CALL_PAYLOAD.to_string(),
                })
            })
            .collect();
        (evidence, observed_at)
    }

    fn evict_encoded_prefix(&mut self, encoded_len: usize) {
        self.buffer.drain(..encoded_len);
        self.base_offset = self
            .base_offset
            .checked_add(encoded_len)
            .expect("validated framing offsets must not overflow");
        while self
            .segments
            .front()
            .is_some_and(|segment| segment.end <= self.base_offset)
        {
            self.segments.pop_front();
        }
        if self.buffer.is_empty() {
            self.base_offset = 0;
        }
    }

    fn discard_buffer(&mut self) {
        self.base_offset = 0;
        self.buffer.clear();
        self.segments.clear();
        self.pending = McpPendingFrame::Unknown;
    }

    fn leading_empty_line_len(buffer: &[u8]) -> usize {
        if buffer.starts_with(b"\r\n") {
            2
        } else if buffer.starts_with(b"\n") {
            1
        } else {
            0
        }
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|candidate| candidate == needle)
    }
}
