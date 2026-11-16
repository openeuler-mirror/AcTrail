use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use semantic_action::{SemanticAction, SemanticEvidence, attr_keys as attrs};

use super::{
    AdmissionId, ContextShape, DetectorKind, PendingAdoption, ProtocolIntent,
    RequestHistoryWitness, ResponseTextWitness,
};
use crate::llm_pipeline::projection::compact::gate::{
    TrajectoryBoundaryGate, TrajectoryFact, TrajectorySlot,
};

pub(in crate::llm_pipeline::projection::compact) struct PendingCall {
    pub(in crate::llm_pipeline::projection::compact) admission_id: AdmissionId,
    pub(in crate::llm_pipeline::projection::compact) request: SemanticAction,
    pub(in crate::llm_pipeline::projection::compact) shape: ContextShape,
    pub(in crate::llm_pipeline::projection::compact) history_source:
        Option<Arc<RequestHistoryWitness>>,
    pub(in crate::llm_pipeline::projection::compact) intent: Option<ProtocolIntent>,
    pub(in crate::llm_pipeline::projection::compact) trajectory: TrajectorySlot,
    pub(in crate::llm_pipeline::projection::compact) source: Option<PredecessorHead>,
    pub(in crate::llm_pipeline::projection::compact) adoption: Option<PendingAdoption>,
}

pub(in crate::llm_pipeline::projection::compact) struct RetainedCall {
    pub(in crate::llm_pipeline::projection::compact) pending: PendingCall,
}

/// A call admitted after its request and successful response are complete.
pub(in crate::llm_pipeline::projection::compact) struct CompletedCallHead {
    pub(in crate::llm_pipeline::projection::compact) request: CompletedRequestHead,
    pub(in crate::llm_pipeline::projection::compact) response_evidence: Vec<SemanticEvidence>,
    pub(in crate::llm_pipeline::projection::compact) call: SemanticAction,
    pub(in crate::llm_pipeline::projection::compact) shape: ContextShape,
    pub(in crate::llm_pipeline::projection::compact) history_source:
        Option<Arc<RequestHistoryWitness>>,
    pub(in crate::llm_pipeline::projection::compact) response_end_time: Option<SystemTime>,
    pub(in crate::llm_pipeline::projection::compact) trajectory: TrajectorySlot,
    pub(in crate::llm_pipeline::projection::compact) response_text: Option<ResponseTextWitness>,
}

#[derive(Clone)]
pub(in crate::llm_pipeline::projection::compact) struct CompletedRequestHead {
    pub(in crate::llm_pipeline::projection::compact) action_id: String,
    pub(in crate::llm_pipeline::projection::compact) start_time: SystemTime,
}

/// Request provenance retained for the next compaction candidate.
#[derive(Clone)]
pub(in crate::llm_pipeline::projection::compact) struct PredecessorHead {
    pub(in crate::llm_pipeline::projection::compact) request: CompletedRequestHead,
    pub(in crate::llm_pipeline::projection::compact) request_evidence: Vec<SemanticEvidence>,
    pub(in crate::llm_pipeline::projection::compact) shape: ContextShape,
    pub(in crate::llm_pipeline::projection::compact) history_source:
        Option<Arc<RequestHistoryWitness>>,
    pub(in crate::llm_pipeline::projection::compact) response_end_time: Option<SystemTime>,
}

pub(in crate::llm_pipeline::projection::compact) struct CompactionCandidate {
    pub(in crate::llm_pipeline::projection::compact) detector: DetectorKind,
    pub(in crate::llm_pipeline::projection::compact) predecessor: Option<PredecessorHead>,
    pub(in crate::llm_pipeline::projection::compact) implementation: CompletedCallHead,
    pub(in crate::llm_pipeline::projection::compact) intent: Option<ProtocolIntent>,
    pub(in crate::llm_pipeline::projection::compact) expires_at: SystemTime,
    pub(in crate::llm_pipeline::projection::compact) inferred_root_only: bool,
}

pub(in crate::llm_pipeline::projection::compact) struct RetiredCandidate {
    pub(in crate::llm_pipeline::projection::compact) action_id: String,
    detector: DetectorKind,
    trajectory: TrajectorySlot,
    inferred_root_only: bool,
}

impl From<&CompactionCandidate> for RetiredCandidate {
    fn from(candidate: &CompactionCandidate) -> Self {
        Self {
            action_id: candidate.implementation.request.action_id.clone(),
            detector: candidate.detector,
            trajectory: candidate.implementation.trajectory.clone(),
            inferred_root_only: candidate.inferred_root_only,
        }
    }
}

impl RetiredCandidate {
    pub(super) fn disproves(&self, current: Option<&TrajectoryFact>) -> bool {
        (self.detector == DetectorKind::ArtifactFlow
            && self
                .trajectory
                .as_ref()
                .is_some_and(|fact| !fact.allows_inferred_candidate(self.inferred_root_only)))
            || (self.trajectory.as_ref().is_some()
                && current.is_some()
                && !TrajectoryBoundaryGate.is_candidate(self.trajectory.as_ref(), current))
    }
}

impl PendingCall {
    /// Retain request metadata and evidence for retries, diagnostics and
    /// trajectory refinement. The classifier selector is the required attribute.
    pub(in crate::llm_pipeline::projection::compact) fn request_metadata(
        request: &SemanticAction,
        classifier_id: &str,
    ) -> SemanticAction {
        SemanticAction {
            action_id: request.action_id.clone(),
            trace_id: request.trace_id,
            kind: request.kind,
            title: request.title.clone(),
            start_time: request.start_time,
            end_time: request.end_time,
            process: request.process,
            status: request.status,
            completeness: request.completeness,
            attributes: BTreeMap::from([(
                attrs::llm_request::CLASSIFIER_ID.to_string(),
                classifier_id.to_string(),
            )]),
            evidence: request.evidence.clone(),
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn has_pending_adoption(&self) -> bool {
        self.adoption.is_some()
    }
}

impl CompactionCandidate {
    pub(in crate::llm_pipeline::projection::compact) fn is_eligible(&self) -> bool {
        self.intent.is_some()
            || self
                .implementation
                .trajectory
                .as_ref()
                .is_none_or(|fact| fact.allows_inferred_candidate(self.inferred_root_only))
    }

    pub(in crate::llm_pipeline::projection::compact) fn applies_to(
        &self,
        request: &SemanticAction,
        current: Option<&TrajectoryFact>,
        gate: &TrajectoryBoundaryGate,
    ) -> bool {
        self.is_eligible()
            && self.expires_at > request.start_time
            && gate.precedes_adoption(
                self.implementation.response_end_time,
                request.start_time,
                self.implementation.trajectory.as_ref(),
                current,
            )
    }
}
