//! Compact payload evidence for LLM projections.

use std::collections::{BTreeMap, BTreeSet};

use model_core::payload::PayloadSegment;
use semantic_action::{SemanticEvidence, SemanticEvidenceKind};

pub(super) fn payload_aggregate_evidence(
    segments: &[&PayloadSegment],
    role: &str,
) -> Vec<SemanticEvidence> {
    let Some(first) = segments.first() else {
        return Vec::new();
    };
    vec![SemanticEvidence {
        kind: SemanticEvidenceKind::PayloadAggregate,
        id: first.segment_id.get(),
        role: role.to_string(),
    }]
}

pub(super) fn insert_payload_span_attributes(
    attributes: &mut BTreeMap<String, String>,
    segments: &[&PayloadSegment],
) {
    let Some(first) = segments.first() else {
        return;
    };
    let last = segments.last().copied().unwrap_or(first);
    attributes.insert(
        "payload.aggregate.first_segment_id".to_string(),
        first.segment_id.get().to_string(),
    );
    attributes.insert(
        "payload.aggregate.last_segment_id".to_string(),
        last.segment_id.get().to_string(),
    );
    attributes.insert(
        "payload.sequence_start".to_string(),
        first.sequence.to_string(),
    );
    attributes.insert(
        "payload.sequence_end".to_string(),
        last.sequence.to_string(),
    );
    attributes.insert(
        "payload.operation_ids".to_string(),
        payload_operation_ids(segments),
    );
    attributes.insert(
        "payload.segment_count".to_string(),
        segments.len().to_string(),
    );
}

fn payload_operation_ids(segments: &[&PayloadSegment]) -> String {
    let mut ids = BTreeSet::new();
    for segment in segments {
        ids.insert(segment.operation_id);
    }
    ids.into_iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}
