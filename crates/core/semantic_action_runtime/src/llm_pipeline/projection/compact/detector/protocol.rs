use super::super::observation::CompactLimitExceeded;
use super::super::state::CompactionCandidate;
use super::super::state::{CompactConfirmation, ProtocolIntent};
use super::{
    CandidateDetection, CompactDetector, DetectionContext, DetectorMatch, PendingAdoption,
};
use crate::llm_pipeline::provider::{ParsedRequestContext, RequestHistorySource};

pub(in crate::llm_pipeline::projection::compact) struct ExplicitProtocolDetector {
    max_items: usize,
    max_artifact_bytes: usize,
}

impl ExplicitProtocolDetector {
    pub(in crate::llm_pipeline::projection::compact) fn new(
        max_items: usize,
        max_artifact_bytes: usize,
    ) -> Self {
        Self {
            max_items,
            max_artifact_bytes,
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn request_intent(
        &self,
        context: &ParsedRequestContext,
    ) -> Option<ProtocolIntent> {
        context.explicit_compaction.clone()
    }

    pub(in crate::llm_pipeline::projection::compact) fn prepare_adoption(
        &self,
        context: &ParsedRequestContext,
        source: &RequestHistorySource,
        prepared: &mut PendingAdoption,
    ) -> Option<CompactLimitExceeded> {
        if context.uses_previous_response {
            return None;
        }
        let item_count = context.input_item_count?;
        if item_count > self.max_items {
            return Some(CompactLimitExceeded::Entries(item_count));
        }
        let artifact = source.opaque_artifact(self.max_items)?;
        if artifact.artifact_bytes > self.max_artifact_bytes {
            return Some(CompactLimitExceeded::Bytes(artifact.artifact_bytes));
        }
        if artifact.has_following_user {
            prepared.opaque = Some(artifact);
        }
        None
    }
}

impl CompactDetector for ExplicitProtocolDetector {
    fn detect(
        &self,
        candidate: &CompactionCandidate,
        context: &mut DetectionContext<'_, '_>,
    ) -> CandidateDetection {
        let Some(adoption) = context.prepared.opaque.as_ref() else {
            return CandidateDetection::NoMatch;
        };
        CandidateDetection::Match(DetectorMatch {
            confirmation: CompactConfirmation::OpaqueArtifact {
                adoption: adoption.clone(),
            },
            awaiting_trajectory: !context.allow_protocol_without_trajectory
                && (context.current.is_none() || candidate.implementation.trajectory.is_none()),
        })
    }
}
