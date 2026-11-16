//! One incremental HTTP/2 framer with ordered protocol events.
use super::{HTTP2_CONNECTION_PREFACE, Http2FrameDecode, decode_http2_frame_state};
use crate::llm_pipeline::transport::buffer::CursorBuffer;
use crate::llm_pipeline::transport::evidence::EvidenceTracker;
use model_core::payload::PayloadSegment;
use std::time::SystemTime;

pub(in crate::llm_pipeline) enum Http2Event {
    Frame {
        observation: crate::Http2FrameObservation,
        evidence: Vec<PayloadSegment>,
    },
    StreamFailure {
        stream_id: u32,
        observed_at: SystemTime,
    },
    ConnectionFailure,
    BufferCapacityExceeded,
}

pub(in crate::llm_pipeline) struct Http2Decoder {
    identity_prefix: String,
    buffer: CursorBuffer,
    base_offset: usize,
    evidence: EvidenceTracker,
}
impl Http2Decoder {
    pub(in crate::llm_pipeline) fn new(identity_prefix: String) -> Self {
        Self {
            identity_prefix,
            buffer: CursorBuffer::default(),
            base_offset: 0,
            evidence: EvidenceTracker::default(),
        }
    }
    pub(in crate::llm_pipeline) fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }
    pub(in crate::llm_pipeline) fn evidence_ranges(&self) -> usize {
        self.evidence.len()
    }
    pub(in crate::llm_pipeline) fn append(&mut self, segment: &PayloadSegment) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(&segment.bytes);
        self.evidence
            .append(start, start + segment.bytes.len(), segment);
    }
    fn observation(
        &self,
        start: usize,
        end: usize,
        frame_type: u8,
        flags: u8,
        stream_id: u32,
        payload: &[u8],
        preface: bool,
    ) -> Option<crate::Http2FrameObservation> {
        let evidence = self.evidence.for_range(start, end);
        let first = evidence.first()?;
        Some(crate::Http2FrameObservation {
            source: crate::Http1MessageSource {
                trace_id: first.trace_id,
                process: first.process,
                source_boundary: first.source_boundary,
                stream_key: first.stream_key.to_string(),
                direction: first.direction,
                observed_at: first.observed_at,
                segment_id: first.segment_id,
                payload_sequence: first.sequence,
            },
            observed_at: evidence.last()?.observed_at,
            payload_segment_ids: evidence.iter().map(|segment| segment.segment_id).collect(),
            frame_id: format!("{}:frame:{}", self.identity_prefix, start),
            message_id: None,
            frame_type,
            flags,
            stream_id,
            payload: std::sync::Arc::new(payload.to_vec()),
            connection_preface: preface,
        })
    }
    pub(in crate::llm_pipeline) fn advance(&mut self, max_buffer_bytes: usize) -> Vec<Http2Event> {
        let mut events = Vec::new();
        let mut cursor = 0;
        if self.base_offset == 0
            && HTTP2_CONNECTION_PREFACE.starts_with(&self.buffer)
            && self.buffer.len() < HTTP2_CONNECTION_PREFACE.len()
        {
            return events;
        }
        if self.buffer.starts_with(HTTP2_CONNECTION_PREFACE) {
            cursor = HTTP2_CONNECTION_PREFACE.len();
            if let Some(observation) = self.observation(
                self.base_offset,
                self.base_offset + cursor,
                0,
                0,
                0,
                HTTP2_CONNECTION_PREFACE,
                true,
            ) {
                events.push(Http2Event::Frame {
                    observation,
                    evidence: Vec::new(),
                });
            }
        }
        loop {
            let bytes = &self.buffer[cursor..];
            if bytes.len() >= 9 {
                let length = (usize::from(bytes[0]) << 16)
                    | (usize::from(bytes[1]) << 8)
                    | usize::from(bytes[2]);
                if length.saturating_add(9) > max_buffer_bytes {
                    self.warn_failure(cursor, max_buffer_bytes, "connection_buffer_limit");
                    events.push(Http2Event::BufferCapacityExceeded);
                    cursor = self.buffer.len();
                    break;
                }
            }
            let frame = match decode_http2_frame_state(bytes) {
                Http2FrameDecode::NeedMore => break,
                Http2FrameDecode::Invalid {
                    encoded_len,
                    stream_id,
                } => {
                    let start = self.base_offset + cursor;
                    let observed_at = self
                        .evidence
                        .for_range(start, start + encoded_len)
                        .last()
                        .map(|source| source.observed_at);
                    if let Some(observed_at) = observed_at {
                        self.warn_failure(cursor, max_buffer_bytes, "invalid_stream_id");
                        if stream_id == 0 {
                            events.push(Http2Event::ConnectionFailure);
                        } else {
                            events.push(Http2Event::StreamFailure {
                                stream_id,
                                observed_at,
                            });
                        }
                    }
                    cursor += encoded_len;
                    continue;
                }
                Http2FrameDecode::Frame(frame) => frame,
            };
            let start = self.base_offset + cursor;
            if let Some(observation) = self.observation(
                start,
                start + frame.encoded_len,
                frame.frame_type,
                frame.flags,
                frame.stream_id,
                frame.payload,
                false,
            ) {
                let evidence = if frame.frame_type == 0 {
                    self.evidence
                        .for_range(start, start + frame.encoded_len)
                        .into_iter()
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                events.push(Http2Event::Frame {
                    observation,
                    evidence,
                });
            }
            cursor += frame.encoded_len;
        }
        self.buffer.release(cursor);
        self.base_offset += cursor;
        self.evidence.evict_before(self.base_offset);
        if self.buffer.is_empty() {
            self.evidence.reset();
        }
        events
    }

    fn warn_failure(&self, cursor: usize, max_buffer_bytes: usize, reason: &'static str) {
        let Some(bytes) = self.buffer.get(cursor..cursor.saturating_add(9)) else {
            return;
        };
        let frame_len =
            (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
        let stream_id = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) & 0x7fff_ffff;
        tracing::warn!(
            reason, frame_len, max_buffer_bytes, frame_type = bytes[3], flags = bytes[4],
            stream_id, offset = self.base_offset.saturating_add(cursor),
            buffered_bytes = self.buffer.len().saturating_sub(cursor),
            connection = %self.identity_prefix, "HTTP/2 framing failed"
        );
    }
}
