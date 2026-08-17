//! Test-only fixtures for calling projection functions directly.
//!
//! The attribute assembly functions take a [`PayloadSegment`] carrying two
//! dozen capture-provenance fields plus the split HTTP message, almost none of
//! which the attributes under test depend on. Constructing that by hand at
//! every call site is what previously pushed tests down to internal pure
//! functions, where they could no longer see whether an attribute was written
//! onto the action at all. These fixtures build the context from the body an
//! operator would actually see on the wire, so a test states only what it is
//! about.
//!
//! The HTTP message is rendered and then split by the real
//! [`split_request`]/[`split_response`] parsers rather than assembled field by
//! field, so a fixture cannot drift into a shape the parsers never produce.

use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadRedactionState,
    PayloadSegment, PayloadSegmentId, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;

use super::http::{HttpRequestParts, HttpResponseParts, split_request, split_response};

const DEFAULT_TRACE_ID: TraceId = TraceId::new(1);
const DEFAULT_PROCESS: ProcessIdentity = ProcessIdentity::new(1);
const DEFAULT_STREAM_KEY: &str = "tls:1:42";
const DEFAULT_OBSERVED_AT_SECS: u64 = 1_700_000_000;

/// A payload segment with coherent defaults, overridable per test.
///
/// Size fields default to the captured bytes and stay consistent with each
/// other, so completeness and status derivation see a whole, successful
/// capture unless a test asks for something else.
pub(super) struct PayloadSegmentBuilder {
    segment: PayloadSegment,
}

pub(super) fn payload_segment() -> PayloadSegmentBuilder {
    PayloadSegmentBuilder {
        segment: PayloadSegment {
            segment_id: PayloadSegmentId::new(1),
            trace_id: DEFAULT_TRACE_ID,
            observed_at: UNIX_EPOCH + Duration::from_secs(DEFAULT_OBSERVED_AT_SECS),
            process: DEFAULT_PROCESS,
            source_boundary: PayloadSourceBoundary::TlsUserSpace,
            content_state: PayloadContentState::Plaintext,
            direction: PayloadDirection::Outbound,
            stream_key: PayloadStreamKey::new(DEFAULT_STREAM_KEY),
            sequence: 0,
            original_size: 0,
            captured_size: 0,
            operation_id: 1,
            operation_offset: 0,
            operation_original_size: 0,
            operation_captured_size: 0,
            operation_completion_state: PayloadOperationCompletionState::Success,
            truncation: PayloadTruncationState::Complete,
            redaction: PayloadRedactionState::NotRequired,
            library: "libssl.so.3".to_string(),
            symbol: "SSL_write".to_string(),
            protocol_hint: None,
            bytes: Vec::new(),
        },
    }
}

impl PayloadSegmentBuilder {
    /// Set the captured bytes, keeping every size field consistent with them.
    pub(super) fn bytes(mut self, bytes: Vec<u8>) -> Self {
        let len = bytes.len() as u64;
        self.segment.bytes = bytes;
        self.segment.original_size = len;
        self.segment.captured_size = len;
        self.segment.operation_original_size = len;
        self.segment.operation_captured_size = len;
        self
    }

    pub(super) fn sequence(mut self, sequence: u64) -> Self {
        self.segment.sequence = sequence;
        self
    }

    pub(super) fn stream_key(mut self, stream_key: &str) -> Self {
        self.segment.stream_key = PayloadStreamKey::new(stream_key);
        self
    }

    pub(super) fn direction(mut self, direction: PayloadDirection) -> Self {
        self.segment.direction = direction;
        self
    }

    pub(super) fn build(self) -> PayloadSegment {
        self.segment
    }
}

/// An outbound LLM request as it reaches the projection: the raw bytes the
/// probe captured, and the same bytes split into HTTP parts.
pub(super) struct HttpRequestFixture {
    pub(super) raw: Vec<u8>,
    pub(super) parts: HttpRequestParts,
}

impl HttpRequestFixture {
    /// A POST carrying `body` as a JSON request to an LLM endpoint.
    pub(super) fn llm_json(body: &str) -> Self {
        let raw = http1_message(
            "POST /v1/messages HTTP/1.1",
            &[
                "host: api.example.test",
                "content-type: application/json",
                &format!("content-length: {}", body.len()),
            ],
            body,
        );
        let parts = split_request(&raw).expect("fixture renders a splittable HTTP request");
        Self { raw, parts }
    }

    /// The single outbound segment that carried this message, ready for a
    /// test to override the capture fields it is actually about.
    pub(super) fn segment_builder(&self) -> PayloadSegmentBuilder {
        payload_segment().bytes(self.raw.clone())
    }
}

/// An inbound LLM response as it reaches the projection.
pub(super) struct HttpResponseFixture {
    pub(super) raw: Vec<u8>,
    pub(super) parts: HttpResponseParts,
}

impl HttpResponseFixture {
    /// A 200 carrying `body` as a complete JSON response.
    pub(super) fn llm_json(body: &str) -> Self {
        let raw = http1_message(
            "HTTP/1.1 200 OK",
            &[
                "content-type: application/json",
                &format!("content-length: {}", body.len()),
            ],
            body,
        );
        let parts = split_response(&raw).expect("fixture renders a splittable HTTP response");
        Self { raw, parts }
    }

    /// The single inbound segment that carried this message, ready for a test
    /// to override the capture fields it is actually about.
    pub(super) fn segment_builder(&self) -> PayloadSegmentBuilder {
        payload_segment()
            .direction(PayloadDirection::Inbound)
            .bytes(self.raw.clone())
    }
}

fn http1_message(start_line: &str, headers: &[&str], body: &str) -> Vec<u8> {
    let mut raw = String::from(start_line);
    raw.push_str("\r\n");
    for header in headers {
        raw.push_str(header);
        raw.push_str("\r\n");
    }
    raw.push_str("\r\n");
    raw.push_str(body);
    raw.into_bytes()
}
