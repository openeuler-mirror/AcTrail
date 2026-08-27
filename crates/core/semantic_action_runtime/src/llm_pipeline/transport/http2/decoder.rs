//! Incremental HTTP/2 frame decoding and DATA demultiplex input.

use model_core::payload::PayloadSegment;
use std::time::SystemTime;

use super::{
    HTTP2_CONNECTION_PREFACE, HTTP2_CONTINUATION_FRAME_TYPE, HTTP2_DATA_FRAME_TYPE,
    HTTP2_FLAG_END_STREAM, HTTP2_HEADERS_FRAME_TYPE, HTTP2_RST_STREAM_FRAME_TYPE, Http2FrameDecode,
    data_payload, decode_http2_frame_state,
};
use crate::llm_pipeline::transport::buffer::CursorBuffer;
use crate::llm_pipeline::transport::evidence::EvidenceTracker;

pub(in crate::llm_pipeline) struct Http2DataEvent {
    pub(in crate::llm_pipeline) stream_id: u32,
    pub(in crate::llm_pipeline) data: Vec<u8>,
    pub(in crate::llm_pipeline) evidence: Vec<PayloadSegment>,
    pub(in crate::llm_pipeline) end_stream: bool,
}

pub(in crate::llm_pipeline) struct Http2EndStreamEvent {
    pub(in crate::llm_pipeline) stream_id: u32,
    pub(in crate::llm_pipeline) observed_at: Option<SystemTime>,
}

pub(in crate::llm_pipeline) struct Http2StreamFailureEvent {
    pub(in crate::llm_pipeline) stream_id: u32,
    pub(in crate::llm_pipeline) observed_at: Option<SystemTime>,
    pub(in crate::llm_pipeline) reset_by_peer: bool,
}

#[derive(Default)]
pub(in crate::llm_pipeline) struct Http2DecodeBatch {
    pub(in crate::llm_pipeline) data: Vec<Http2DataEvent>,
    pub(in crate::llm_pipeline) ended: Vec<Http2EndStreamEvent>,
    pub(in crate::llm_pipeline) failures: Vec<Http2StreamFailureEvent>,
    pub(in crate::llm_pipeline) connection_failures: usize,
}

#[derive(Default)]
pub(in crate::llm_pipeline) struct Http2Decoder {
    buffer: CursorBuffer,
    base_offset: usize,
    evidence: EvidenceTracker,
}

impl Http2Decoder {
    pub(in crate::llm_pipeline) fn from_buffer(
        buffer: Vec<u8>,
        base_offset: usize,
        evidence: EvidenceTracker,
    ) -> Self {
        Self {
            buffer: CursorBuffer::from_vec(buffer),
            base_offset,
            evidence,
        }
    }

    pub(in crate::llm_pipeline) fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }

    pub(in crate::llm_pipeline) fn end_offset(&self) -> usize {
        self.base_offset.saturating_add(self.buffer.len())
    }

    pub(in crate::llm_pipeline) fn evidence_ranges(&self) -> usize {
        self.evidence.len()
    }

    pub(in crate::llm_pipeline) fn append(&mut self, segment: &PayloadSegment) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(&segment.bytes);
        let end = self.base_offset + self.buffer.len();
        self.evidence.append(start, end, segment);
    }

    pub(in crate::llm_pipeline) fn advance(&mut self) -> Http2DecodeBatch {
        let mut batch = Http2DecodeBatch::default();
        let mut cursor = 0;
        if self.buffer.starts_with(HTTP2_CONNECTION_PREFACE) {
            cursor = HTTP2_CONNECTION_PREFACE.len();
        }
        loop {
            let frame = match decode_http2_frame_state(&self.buffer[cursor..]) {
                Http2FrameDecode::NeedMore => break,
                Http2FrameDecode::Invalid {
                    encoded_len,
                    stream_id,
                } => {
                    let frame_start = self.base_offset + cursor;
                    let frame_end = frame_start + encoded_len;
                    let observed_at = self
                        .evidence
                        .for_range(frame_start, frame_end)
                        .last()
                        .map(|segment| segment.observed_at);
                    if stream_id == 0 {
                        batch.connection_failures = batch.connection_failures.saturating_add(1);
                    } else {
                        batch.failures.push(Http2StreamFailureEvent {
                            stream_id,
                            observed_at,
                            reset_by_peer: false,
                        });
                    }
                    cursor += encoded_len;
                    continue;
                }
                Http2FrameDecode::Frame(frame) => frame,
            };
            let encoded_len = frame.encoded_len;
            let frame_start = self.base_offset + cursor;
            let frame_end = frame_start + encoded_len;
            let end_stream = frame.flags & HTTP2_FLAG_END_STREAM != 0;
            match frame.frame_type {
                HTTP2_DATA_FRAME_TYPE => {
                    if let Some(data) = data_payload(frame.flags, frame.payload) {
                        let evidence = self
                            .evidence
                            .for_range(frame_start, frame_end)
                            .into_iter()
                            .cloned()
                            .collect();
                        batch.data.push(Http2DataEvent {
                            stream_id: frame.stream_id,
                            data: data.to_vec(),
                            evidence,
                            end_stream,
                        });
                    } else if let Some(observed_at) = self
                        .evidence
                        .for_range(frame_start, frame_end)
                        .last()
                        .map(|segment| segment.observed_at)
                    {
                        batch.failures.push(Http2StreamFailureEvent {
                            stream_id: frame.stream_id,
                            observed_at: Some(observed_at),
                            reset_by_peer: false,
                        });
                    }
                }
                HTTP2_HEADERS_FRAME_TYPE | HTTP2_CONTINUATION_FRAME_TYPE if end_stream => {
                    batch.ended.push(Http2EndStreamEvent {
                        stream_id: frame.stream_id,
                        observed_at: self
                            .evidence
                            .for_range(frame_start, frame_end)
                            .last()
                            .map(|segment| segment.observed_at),
                    });
                }
                HTTP2_RST_STREAM_FRAME_TYPE => {
                    if frame.payload.len() == 4 {
                        batch.failures.push(Http2StreamFailureEvent {
                            stream_id: frame.stream_id,
                            observed_at: self
                                .evidence
                                .for_range(frame_start, frame_end)
                                .last()
                                .map(|segment| segment.observed_at),
                            reset_by_peer: true,
                        });
                    } else {
                        batch.failures.push(Http2StreamFailureEvent {
                            stream_id: frame.stream_id,
                            observed_at: self
                                .evidence
                                .for_range(frame_start, frame_end)
                                .last()
                                .map(|segment| segment.observed_at),
                            reset_by_peer: false,
                        });
                    }
                }
                _ => {}
            }
            cursor += encoded_len;
        }
        self.release(cursor);
        batch
    }

    fn release(&mut self, consumed: usize) {
        if consumed == 0 {
            return;
        }
        let Some(global_end) = self.base_offset.checked_add(consumed) else {
            tracing::warn!(consumed, "refused overflowing HTTP/2 buffer release");
            return;
        };
        if !self.buffer.release(consumed) {
            tracing::warn!(
                consumed,
                buffered_bytes = self.buffer.len(),
                "refused out-of-range HTTP/2 buffer release"
            );
            return;
        }
        self.base_offset = global_end;
        self.evidence.evict_before(global_end);
        if self.buffer.is_empty() {
            self.evidence.reset();
        }
    }
}
