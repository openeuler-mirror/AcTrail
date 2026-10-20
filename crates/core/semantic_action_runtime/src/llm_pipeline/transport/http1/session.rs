//! One authoritative HTTP/1 message decoder and its source coordinates.

use std::time::SystemTime;

use model_core::payload::{PayloadDirection, PayloadSegment};

use crate::llm_pipeline::transport::buffer::CursorBuffer;
use crate::llm_pipeline::transport::evidence::{EvidenceCursor, EvidenceSnapshot, EvidenceTracker};
use crate::{
    Http1AbortReason, Http1MessageEnd, Http1MessageEvent, Http1MessageHead, Http1MessageSource,
};

use super::{DecodedHttp1Message, Http1DecodeFailure, Http1Decoder, Http1Direction};

pub(in crate::llm_pipeline) enum Http1Update<'a> {
    Head(&'a Http1MessageHead),
    Content {
        head: &'a Http1MessageHead,
        message: DecodedHttp1Message,
        wire: &'a [u8],
        segments: Vec<&'a PayloadSegment>,
        evidence: &'a EvidenceSnapshot,
        message_start: usize,
        force_terminal: bool,
    },
}

pub(in crate::llm_pipeline) struct Http1Upgrade {
    pub(in crate::llm_pipeline) request_message_id: Option<String>,
    pub(in crate::llm_pipeline) head: Http1MessageHead,
    pub(in crate::llm_pipeline) tails: Vec<PayloadSegment>,
}

/// A head callback returns the request method for a response, when known.
/// The method is applied before reading body bytes (HEAD and CONNECT).
pub(in crate::llm_pipeline) struct Http1Session {
    decoder: Http1Decoder,
    wire: CursorBuffer,
    base_offset: usize,
    evidence: EvidenceTracker,
    cursor: Option<EvidenceCursor>,
    head: Option<Http1MessageHead>,
    published_body_len: usize,
    max_bytes: usize,
    max_ranges: usize,
    upgraded: bool,
    awaiting_request: bool,
    observed_at: Option<SystemTime>,
    upgrade: Option<(Http1MessageHead, PayloadSegment)>,
}

impl Http1Session {
    pub(in crate::llm_pipeline) fn active_message_id(&self) -> Option<&str> {
        self.head.as_ref().map(|head| head.message_id.as_str())
    }
    pub(in crate::llm_pipeline) fn has_unclassified_tail(&self) -> bool {
        self.awaiting_request || (self.head.is_none() && !self.wire.is_empty())
    }
    pub(in crate::llm_pipeline) fn new(
        direction: PayloadDirection,
        max_bytes: usize,
        max_ranges: usize,
        max_decoded_bytes: usize,
    ) -> Self {
        Self {
            decoder: Http1Decoder::new(
                match direction {
                    PayloadDirection::Outbound => Http1Direction::Request,
                    PayloadDirection::Inbound => Http1Direction::Response,
                },
                max_bytes,
            )
            .with_decoded_limit(max_decoded_bytes),
            wire: CursorBuffer::default(),
            base_offset: 0,
            evidence: EvidenceTracker::default(),
            cursor: None,
            head: None,
            published_body_len: 0,
            max_bytes,
            max_ranges,
            upgraded: false,
            awaiting_request: false,
            observed_at: None,
            upgrade: None,
        }
    }

    pub(in crate::llm_pipeline) fn advance(
        &mut self,
        segment: &PayloadSegment,
        next_message: &mut u64,
        consume: impl FnMut(Http1Update<'_>) -> Option<String>,
    ) -> Vec<Http1MessageEvent> {
        if self.upgraded {
            return Vec::new();
        }
        if self.wire.len().saturating_add(segment.bytes.len()) > self.max_bytes
            || self.evidence.len() >= self.max_ranges
        {
            let mut events = self.finalize(next_message, consume);
            events.extend(self.abort(Http1AbortReason::CapacityExceeded));
            return events;
        }
        let start = self.base_offset + self.wire.len();
        self.wire.extend_from_slice(&segment.bytes);
        self.evidence
            .append(start, start + segment.bytes.len(), segment);
        self.observed_at = Some(segment.observed_at);
        self.resume(next_message, None, consume)
    }

    pub(in crate::llm_pipeline) fn resume(
        &mut self,
        next_message: &mut u64,
        eof_at: Option<SystemTime>,
        consume: impl FnMut(Http1Update<'_>) -> Option<String>,
    ) -> Vec<Http1MessageEvent> {
        if eof_at.is_some() && !self.evidence.capture_tail_complete() {
            let mut events = self.process(next_message, None, true, consume);
            events.extend(self.abort(Http1AbortReason::CaptureIncomplete));
            return events;
        }
        self.process(next_message, eof_at, eof_at.is_some(), consume)
    }

    pub(in crate::llm_pipeline) fn finalize(
        &mut self,
        next_message: &mut u64,
        consume: impl FnMut(Http1Update<'_>) -> Option<String>,
    ) -> Vec<Http1MessageEvent> {
        self.process(next_message, None, true, consume)
    }

    fn process(
        &mut self,
        next_message: &mut u64,
        eof_at: Option<SystemTime>,
        force_terminal: bool,
        mut consume: impl FnMut(Http1Update<'_>) -> Option<String>,
    ) -> Vec<Http1MessageEvent> {
        let mut output = Vec::new();
        loop {
            if self.head.is_none() {
                let message = match self.decoder.head(&self.wire) {
                    Ok(Some(message)) => message,
                    Ok(None) => break,
                    Err(_) => {
                        output.extend(self.abort(Http1AbortReason::DecodeFailure));
                        break;
                    }
                };
                let Some(serial) = next_message.checked_add(1) else {
                    output.extend(self.abort(Http1AbortReason::CapacityExceeded));
                    break;
                };
                *next_message = serial;
                let sources = self
                    .evidence
                    .for_range(self.base_offset, self.base_offset + message.encoded_len);
                let Some(first) = sources.first() else {
                    break;
                };
                let head = Http1MessageHead {
                    message_id: format!("trace:{}:http.message:{}", first.trace_id.get(), serial),
                    source: Http1MessageSource {
                        trace_id: first.trace_id,
                        process: first.process,
                        source_boundary: first.source_boundary,
                        stream_key: first.stream_key.to_string(),
                        direction: first.direction,
                        observed_at: first.observed_at,
                        segment_id: first.segment_id,
                        payload_sequence: first.sequence,
                    },
                    protocol: message.protocol,
                    method: message.method,
                    authority: message.authority,
                    path: message.path,
                    status_code: message.status_code,
                    reason: message.reason,
                    headers_text: message.headers_text,
                    declared_body_len: message.declared_body_len,
                };
                if let Some(method) = consume(Http1Update::Head(&head)) {
                    self.upgraded = self.decoder.apply_request_method(&method);
                } else if head.status_code.is_some() {
                    self.awaiting_request = true;
                }
                output.push(Http1MessageEvent::Head(head.clone()));
                self.head = Some(head);
                self.cursor = Some(self.evidence.cursor(self.base_offset));
            }
            if self.awaiting_request {
                break;
            }
            let mut message = match self.decoder.advance(&self.wire, eof_at.is_some()) {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(failure) => {
                    output.extend(self.abort(match failure {
                        Http1DecodeFailure::BufferCapacity => Http1AbortReason::CapacityExceeded,
                        _ => Http1AbortReason::DecodeFailure,
                    }));
                    break;
                }
            };
            let head = self.head.as_ref().expect("confirmed head owns message");
            message.message_id = Some(head.message_id.clone());
            let framing_failure = message.framing_failure;
            let complete = message.complete;
            // Only close-delimited bodies end at EOF. Framed messages retain
            // their payload completion time, even when processed during close.
            let eof_end_at = if message.body_boundary_known {
                None
            } else {
                eof_at
            };
            let encoded_len = message.encoded_len;
            if let Some(body) = message.body.bytes(&self.wire) {
                if let Some(delta) = body.get(self.published_body_len..) {
                    if !delta.is_empty() {
                        output.push(Http1MessageEvent::Body {
                            message_id: head.message_id.clone(),
                            bytes: delta.to_vec(),
                        });
                    }
                }
                self.published_body_len = body.len();
            }
            let segments = if head.method.is_some() && (complete || force_terminal) {
                self.evidence
                    .for_range(self.base_offset, self.base_offset + encoded_len)
            } else {
                Vec::new()
            };
            let cursor = self.cursor.as_mut().expect("head owns evidence cursor");
            self.evidence
                .advance_cursor(cursor, self.base_offset + encoded_len);
            let snapshot = cursor.snapshot();
            consume(Http1Update::Content {
                head,
                message,
                wire: &self.wire[..encoded_len],
                segments,
                evidence: &snapshot,
                message_start: self.base_offset,
                force_terminal: force_terminal || framing_failure.is_some(),
            });
            if let Some(failure) = framing_failure {
                output.extend(self.abort(match failure {
                    Http1DecodeFailure::BufferCapacity => Http1AbortReason::CapacityExceeded,
                    _ => Http1AbortReason::DecodeFailure,
                }));
                break;
            }
            if !complete {
                break;
            }
            output.push(Http1MessageEvent::End(Http1MessageEnd {
                message_id: head.message_id.clone(),
                observed_at: eof_end_at
                    .or(self.observed_at)
                    .unwrap_or(head.source.observed_at),
            }));
            if self.upgraded {
                if let Some(source) = snapshot.last.as_ref() {
                    self.upgrade = Some((head.clone(), source.clone()));
                }
            }
            self.head = None;
            self.published_body_len = 0;
            self.base_offset += encoded_len;
            self.wire.release(encoded_len);
            self.cursor = None;
            self.evidence.evict_before(self.base_offset);
            self.decoder.reset();
            if self.upgraded || self.wire.is_empty() {
                break;
            }
        }
        output
    }

    pub(in crate::llm_pipeline) fn abort(
        &mut self,
        reason: Http1AbortReason,
    ) -> Vec<Http1MessageEvent> {
        let output = self
            .head
            .take()
            .map(|head| Http1MessageEvent::Abort {
                message_id: head.message_id,
                reason,
                observed_at: self.observed_at.unwrap_or(head.source.observed_at),
            })
            .into_iter()
            .collect();
        self.base_offset += self.wire.len();
        self.wire = CursorBuffer::default();
        self.cursor = None;
        self.evidence.reset();
        self.published_body_len = 0;
        self.decoder.reset();
        self.awaiting_request = false;
        output
    }

    pub(in crate::llm_pipeline) fn take_upgrade(&mut self) -> Option<Http1Upgrade> {
        let (head, _) = self.upgrade.take()?;
        let tails = self.evidence.slice_payloads(self.base_offset, &self.wire);
        self.wire = CursorBuffer::default();
        Some(Http1Upgrade {
            request_message_id: None,
            head,
            tails,
        })
    }

    pub(in crate::llm_pipeline) fn bind_request(
        &mut self,
        response_id: &str,
        method: &str,
    ) -> bool {
        if self
            .head
            .as_ref()
            .is_some_and(|head| head.message_id == response_id)
        {
            self.upgraded = self.decoder.apply_request_method(method);
            self.awaiting_request = false;
            return true;
        }
        false
    }
}
