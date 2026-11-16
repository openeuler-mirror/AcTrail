//! The content decoder owns body bytes; evidence retains wire DATA coordinates.
use super::{Http2BodyAssembly, Http2StreamAssembly};
use crate::Http1MessageHead;
use crate::llm_pipeline::assembly::router::{AssemblyLimits, AssemblyResetReason};
use crate::llm_pipeline::transport::content_decoder::{ContentDecodeFailure, DecoderFactory};
use model_core::payload::PayloadSegment;
use std::time::SystemTime;

impl Http2StreamAssembly {
    fn failure(failure: ContentDecodeFailure) -> AssemblyResetReason {
        match failure {
            ContentDecodeFailure::BufferCapacity => AssemblyResetReason::BufferBytesExceeded,
            ContentDecodeFailure::InvalidContentEncoding => {
                AssemblyResetReason::ContentDecodeFailed
            }
        }
    }
}

impl Http2BodyAssembly {
    pub(in crate::llm_pipeline) fn begin_message(
        &mut self,
        stream_id: u32,
        head: &Http1MessageHead,
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        let stream = self.streams.entry(stream_id).or_default();
        stream.message_id = Some(head.message_id.clone());
        stream.head = Some(head.clone());
        match DecoderFactory::create(&head.headers_text, limits.max_buffer_bytes) {
            Ok(body) => {
                stream.body = Some(body);
                None
            }
            Err(failure) => Some(Http2StreamAssembly::failure(failure)),
        }
    }

    pub(in crate::llm_pipeline) fn append_data(
        &mut self,
        stream_id: u32,
        _message_id: &str,
        data: &[u8],
        evidence: &[PayloadSegment],
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        let Some(stream) = self.streams.get_mut(&stream_id) else {
            return Some(AssemblyResetReason::ContentDecodeFailed);
        };
        if stream.discarded {
            return None;
        }
        if self
            .retained_ranges
            .checked_add(evidence.len())
            .is_none_or(|size| size > limits.max_segment_ranges)
        {
            return Some(AssemblyResetReason::SegmentRangesExceeded);
        }
        if stream.plain.encoded_len.checked_add(data.len()).is_none() {
            return Some(AssemblyResetReason::BufferBytesExceeded);
        }
        let before = stream.retention_footprint();
        stream.plain.append_evidence(data.len(), evidence);
        let failure = match stream.body.as_mut() {
            Some(body) => body.append(data).err().map(Http2StreamAssembly::failure),
            None => Some(AssemblyResetReason::ContentDecodeFailed),
        };
        let after = stream.retention_footprint();
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub(before.bytes)
            .saturating_add(after.bytes);
        self.retained_ranges = self
            .retained_ranges
            .saturating_sub(before.ranges)
            .saturating_add(after.ranges);
        self.dirty.insert(stream_id);
        failure.or_else(|| {
            (self.retained_bytes > limits.max_buffer_bytes)
                .then_some(AssemblyResetReason::BufferBytesExceeded)
        })
    }

    pub(in crate::llm_pipeline) fn end_stream(
        &mut self,
        stream_id: u32,
        at: SystemTime,
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        let stream = self.streams.get_mut(&stream_id)?;
        stream.end_stream = true;
        stream.end_stream_observed_at = Some(at);
        self.dirty.insert(stream_id);
        let before = stream.retention_footprint();
        let failure = stream
            .body
            .as_mut()
            .and_then(|body| body.finish().err())
            .map(Http2StreamAssembly::failure);
        let after = stream.retention_footprint();
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub(before.bytes)
            .saturating_add(after.bytes);
        failure.or_else(|| {
            (self.retained_bytes > limits.max_buffer_bytes)
                .then_some(AssemblyResetReason::BufferBytesExceeded)
        })
    }
}
