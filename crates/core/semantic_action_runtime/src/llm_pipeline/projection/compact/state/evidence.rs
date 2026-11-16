use super::{AdoptionChoices, PairEvidence, PreparedCarrierSnapshot};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use crate::llm_pipeline::provider::{RequestHistoryObservation, RequestHistorySource};

pub(in crate::llm_pipeline::projection::compact) use crate::llm_pipeline::provider::{
    ExplicitCompactionIntent as ProtocolIntent, OpaqueContextArtifact as OpaqueCompactReplacement,
    RequestContextShape as ContextShape,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(in crate::llm_pipeline::projection::compact) enum DetectorKind {
    ExplicitProtocol,
    ArtifactFlow,
}

impl DetectorKind {
    /// Authoritative protocol evidence fixes compaction candidate's strategy at admission.
    pub(in crate::llm_pipeline::projection::compact) fn select(
        intent: Option<&ProtocolIntent>,
    ) -> Self {
        if intent.is_some() {
            Self::ExplicitProtocol
        } else {
            Self::ArtifactFlow
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::llm_pipeline::projection::compact) struct ArtifactFlow {
    pub(in crate::llm_pipeline::projection::compact) chars: usize,
    pub(in crate::llm_pipeline::projection::compact) bytes: usize,
    pub(in crate::llm_pipeline::projection::compact) carrier_item: usize,
    pub(in crate::llm_pipeline::projection::compact) partial_summary: bool,
}

pub(in crate::llm_pipeline::projection::compact) enum ArtifactFlowObservation {
    Present(ArtifactFlow),
    Structured(ArtifactFlow),
    WorkLimitExceeded,
}

/// Pool pending adoption owns the bounded immutable source. Its carrier view is materialized
/// once, when an applicable compaction candidate exists, and released together with this entry.
pub(in crate::llm_pipeline::projection::compact) struct PendingAdoption {
    pub(in crate::llm_pipeline::projection::compact) snapshot: Option<PreparedCarrierSnapshot>,
    pub(in crate::llm_pipeline::projection::compact) snapshot_prepared: bool,
    pub(in crate::llm_pipeline::projection::compact) opaque: Option<OpaqueCompactReplacement>,
    pub(in crate::llm_pipeline::projection::compact) history_source:
        Option<Arc<RequestHistoryWitness>>,
    pub(in crate::llm_pipeline::projection::compact) expires_at: SystemTime,
    pub(in crate::llm_pipeline::projection::compact) matches: HashMap<String, PairEvidence>,
    pub(in crate::llm_pipeline::projection::compact) initialized: bool,
    pub(in crate::llm_pipeline::projection::compact) choices: AdoptionChoices,
}

impl PendingAdoption {
    pub(in crate::llm_pipeline::projection::compact) fn confirmation(
        &self,
        action_id: &str,
    ) -> Option<CompactConfirmation> {
        let pair = self.matches.get(action_id)?;
        if pair.status != super::PairStatus::Ready {
            return None;
        }
        match pair.detector {
            DetectorKind::ExplicitProtocol => Some(CompactConfirmation::OpaqueArtifact {
                adoption: self.opaque.clone()?,
            }),
            DetectorKind::ArtifactFlow => match pair.content? {
                ArtifactContentMatch::Match(flow) => Some(CompactConfirmation::ArtifactFlow {
                    adoption_shape: self.snapshot.as_ref()?.snapshot.shape,
                    flow,
                }),
                _ => None,
            },
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn supports(
        &self,
        detector: DetectorKind,
    ) -> bool {
        match detector {
            DetectorKind::ExplicitProtocol => self.opaque.is_some(),
            DetectorKind::ArtifactFlow => {
                self.history_source.is_some()
                    && (!self.snapshot_prepared || self.snapshot.is_some())
            }
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn can_wait(
        &self,
        observed_at: SystemTime,
    ) -> bool {
        self.expires_at > observed_at
            && (self.supports(DetectorKind::ExplicitProtocol)
                || self.supports(DetectorKind::ArtifactFlow))
    }
}

/// One immutable request source shared by its open call, compaction candidate, pending adoption and provenance.
/// Observations, including bounded failures, are released with the last owner.
pub(in crate::llm_pipeline::projection::compact) struct RequestHistoryWitness {
    pub(in crate::llm_pipeline::projection::compact) source: Arc<RequestHistorySource>,
    pub(in crate::llm_pipeline::projection::compact) snapshot: OnceLock<RequestHistoryObservation>,
}

impl RequestHistoryWitness {
    pub(in crate::llm_pipeline::projection::compact) fn new(
        source: Arc<RequestHistorySource>,
    ) -> Self {
        Self {
            source,
            snapshot: OnceLock::new(),
        }
    }
}

/// Immutable content evidence is reused when a source's trajectory becomes known.
#[derive(Clone, Copy)]
pub(in crate::llm_pipeline::projection::compact) enum ArtifactContentMatch {
    NoMatch,
    Match(ArtifactFlow),
    WorkLimitExceeded,
}

#[derive(Clone)]
pub(in crate::llm_pipeline::projection::compact) enum CompactConfirmation {
    ExplicitProtocol,
    ArtifactFlow {
        adoption_shape: ContextShape,
        flow: ArtifactFlow,
    },
    OpaqueArtifact {
        adoption: OpaqueCompactReplacement,
    },
}

pub(in crate::llm_pipeline::projection::compact) struct DetectorMatch {
    pub(in crate::llm_pipeline::projection::compact) confirmation: CompactConfirmation,
    pub(in crate::llm_pipeline::projection::compact) awaiting_trajectory: bool,
}

pub(in crate::llm_pipeline::projection::compact) enum CandidateDetection {
    NoMatch,
    Match(DetectorMatch),
    WorkLimitExceeded,
}

pub(in crate::llm_pipeline::projection::compact) enum DetectionOutcome {
    Unconfirmed,
    Confirmed,
}
