use export_core::{PayloadReference, PayloadReferenceMetadata};
use model_core::diagnostics::DiagnosticRecord;
use model_core::ids::TraceId;
use model_core::payload::{PayloadSegment, PayloadSegmentId};
use semantic_action::{SemanticAction, SemanticEvidenceKind};
use std::collections::BTreeMap;

use crate::{DeliveryFailureKind, DeliveryReport, RecordingError};

#[derive(Default)]
pub(crate) struct SemanticActionPublication {
    pub(crate) delivery: DeliveryReport,
    pub(crate) diagnostics: Vec<DiagnosticRecord>,
}

impl SemanticActionPublication {
    pub(crate) fn payload_references(
        actions: &[SemanticAction],
        segments: &[PayloadSegment],
    ) -> Vec<PayloadReference> {
        let mut references = BTreeMap::<(TraceId, PayloadSegmentId), PayloadReference>::new();
        for action in actions {
            for evidence in &action.evidence {
                if evidence.kind == SemanticEvidenceKind::PayloadSegment {
                    let segment_id = PayloadSegmentId::new(evidence.id);
                    references
                        .entry((action.trace_id, segment_id))
                        .or_insert(PayloadReference {
                            segment_id,
                            trace_id: action.trace_id,
                            metadata: None,
                        });
                }
            }
        }
        for segment in segments {
            references.insert(
                (segment.trace_id, segment.segment_id),
                PayloadReference {
                    segment_id: segment.segment_id,
                    trace_id: segment.trace_id,
                    metadata: Some(PayloadReferenceMetadata {
                        captured_size: segment.captured_size,
                        original_size: segment.original_size,
                        redaction: segment.redaction,
                        truncation: segment.truncation,
                    }),
                },
            );
        }
        references.into_values().collect()
    }

    pub(crate) fn record(&mut self, kind: DeliveryFailureKind, result: Result<(), RecordingError>) {
        self.delivery.record(kind, result);
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.delivery.extend(other.delivery);
        self.diagnostics.extend(other.diagnostics);
    }
}
