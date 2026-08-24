//! Semantic retention and assembled-payload policy application.

use model_core::payload::{PayloadRedactionState, PayloadSegment};

/// Build a semantic-exchange payload record from the assembled message bytes.
///
/// Emitted only when transport segments are not persisted (L4 payload retention
/// disabled): the exchange's assembled request/response bytes are written as a
/// single payload row reusing the first segment id, which the action payload
/// evidence already references.
pub(in crate::llm_pipeline) fn semantic_payload_draft(
    first: &PayloadSegment,
    assembled_bytes: &[u8],
) -> PayloadSegment {
    let size = u64::try_from(assembled_bytes.len()).unwrap_or(u64::MAX);
    PayloadSegment {
        segment_id: first.segment_id,
        trace_id: first.trace_id,
        observed_at: first.observed_at,
        process: first.process.clone(),
        source_boundary: first.source_boundary,
        content_state: first.content_state,
        direction: first.direction,
        stream_key: first.stream_key.clone(),
        sequence: first.sequence,
        original_size: size,
        captured_size: size,
        operation_id: first.operation_id,
        operation_offset: 0,
        operation_original_size: size,
        operation_captured_size: size,
        operation_completion_state: first.operation_completion_state,
        truncation: first.truncation,
        redaction: PayloadRedactionState::NotRequired,
        library: first.library.clone(),
        symbol: first.symbol.clone(),
        protocol_hint: first.protocol_hint.clone(),
        bytes: assembled_bytes.to_vec(),
    }
}
