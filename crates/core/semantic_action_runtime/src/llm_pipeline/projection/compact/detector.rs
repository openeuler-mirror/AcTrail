use semantic_action::SemanticAction;

use super::gate::TrajectoryFact;
use super::history::{HistoryContinuationFilter, HistoryMaterializer};
use super::state::{
    ArtifactFlow, ArtifactFlowObservation, CandidateDetection, CompactionCandidate, ContextShape,
    DetectorKind, DetectorMatch, PendingAdoption,
};
use model_core::diagnostics::LlmPipelineDiagnostic;

mod artifact;
mod flow;
mod json_text;
mod protocol;
mod rolling;

pub(super) use crate::llm_pipeline::provider::CarrierSnapshotObservation;
pub(super) use flow::ArtifactFlowDetector;
pub(super) use protocol::ExplicitProtocolDetector;

impl DetectorKind {
    pub(super) fn detector<'a>(
        self,
        protocol: &'a ExplicitProtocolDetector,
        artifact: &'a ArtifactFlowDetector,
    ) -> &'a dyn CompactDetector {
        match self {
            Self::ExplicitProtocol => protocol,
            Self::ArtifactFlow => artifact,
        }
    }
}

pub(super) struct DetectionContext<'a, 'h> {
    pub(super) history: &'a HistoryContinuationFilter,
    pub(super) materializer: &'a mut HistoryMaterializer<'h>,
    pub(super) allow_protocol_without_trajectory: bool,
    pub(super) request: &'a SemanticAction,
    pub(super) current: Option<&'a TrajectoryFact>,
    pub(super) prepared: &'a mut PendingAdoption,
    pub(super) diagnostics: &'a mut Vec<LlmPipelineDiagnostic>,
}

/// A strategy evaluates one preselected compaction candidate. Pool ownership, arbitration and
/// event emission belong to the coordinator, independently of Agent identity.
pub(super) trait CompactDetector {
    fn detect(
        &self,
        candidate: &CompactionCandidate,
        context: &mut DetectionContext<'_, '_>,
    ) -> CandidateDetection;
}
