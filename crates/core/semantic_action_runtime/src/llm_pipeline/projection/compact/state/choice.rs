use super::{AdmissionId, ArtifactContentMatch, CompactScopeKey, DetectorKind};

#[derive(Clone)]
pub(in crate::llm_pipeline::projection::compact) struct CandidateSource {
    pub(in crate::llm_pipeline::projection::compact) owner: CompactScopeKey,
    pub(in crate::llm_pipeline::projection::compact) admission_id: AdmissionId,
    pub(in crate::llm_pipeline::projection::compact) action_id: String,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(in crate::llm_pipeline::projection::compact) enum PairStatus {
    #[default]
    NoMatch,
    Pending,
    Limited,
    Ready,
}

impl PairStatus {
    fn blocks(self) -> bool {
        matches!(self, Self::Pending | Self::Limited)
    }
}

pub(in crate::llm_pipeline::projection::compact) struct PairEvidence {
    pub(in crate::llm_pipeline::projection::compact) detector: DetectorKind,
    pub(in crate::llm_pipeline::projection::compact) content: Option<ArtifactContentMatch>,
    pub(in crate::llm_pipeline::projection::compact) status: PairStatus,
}

impl PairEvidence {
    pub(in crate::llm_pipeline::projection::compact) fn new(detector: DetectorKind) -> Self {
        Self {
            detector,
            content: None,
            status: PairStatus::NoMatch,
        }
    }
}

/// Derived from pair transitions. Proven ambiguity and lost evidence survive pool reclamation.
#[derive(Default)]
pub(in crate::llm_pipeline::projection::compact) struct SourceChoice {
    source: Option<CandidateSource>,
    ready: usize,
    pending: usize,
    unavailable: bool,
    ambiguous: bool,
    reported: bool,
}

impl SourceChoice {
    pub(in crate::llm_pipeline::projection::compact) fn update(
        &mut self,
        previous: PairStatus,
        next: PairStatus,
        source: CandidateSource,
    ) {
        if previous == next {
            return;
        }
        if previous.blocks() {
            self.pending = self.pending.saturating_sub(1);
        }
        if previous == PairStatus::Ready {
            self.ready = self.ready.saturating_sub(1);
            self.unavailable = true;
            self.source = None;
        }
        if next.blocks() {
            self.pending = self.pending.saturating_add(1);
        }
        if next == PairStatus::Ready {
            self.ready = self.ready.saturating_add(1);
            if self.ready > 1 {
                self.ambiguous = true;
                self.source = None;
            } else if !self.ambiguous && !self.unavailable {
                self.source = Some(source);
            }
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn discard(&mut self, status: PairStatus) {
        if status != PairStatus::NoMatch {
            self.unavailable = true;
            self.source = None;
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn exclude(&mut self, status: PairStatus) {
        if status.blocks() {
            self.pending = self.pending.saturating_sub(1);
        }
        if status == PairStatus::Ready {
            self.discard(status);
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn has_evidence(&self) -> bool {
        self.ready > 0 || self.pending > 0 || self.unavailable || self.ambiguous
    }

    pub(in crate::llm_pipeline::projection::compact) fn unique(&self) -> Option<&CandidateSource> {
        if self.pending > 0 || self.unavailable || self.ambiguous {
            return None;
        }
        self.source.as_ref()
    }

    pub(in crate::llm_pipeline::projection::compact) fn take_ambiguity(&mut self) -> Option<usize> {
        if !self.ambiguous || self.reported {
            return None;
        }
        self.reported = true;
        Some(self.ready)
    }
}

#[derive(Default)]
pub(in crate::llm_pipeline::projection::compact) struct AdoptionChoices {
    protocol: SourceChoice,
    artifact: SourceChoice,
}

impl AdoptionChoices {
    pub(in crate::llm_pipeline::projection::compact) fn can_resolve(
        &self,
        kind: DetectorKind,
    ) -> bool {
        if self.protocol.unavailable || self.protocol.ambiguous {
            return false;
        }
        match kind {
            DetectorKind::ExplicitProtocol => true,
            DetectorKind::ArtifactFlow => !self.artifact.unavailable && !self.artifact.ambiguous,
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn group(
        &mut self,
        kind: DetectorKind,
    ) -> &mut SourceChoice {
        match kind {
            DetectorKind::ExplicitProtocol => &mut self.protocol,
            DetectorKind::ArtifactFlow => &mut self.artifact,
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn selected(&mut self) -> &mut SourceChoice {
        if self.protocol.has_evidence() {
            &mut self.protocol
        } else {
            &mut self.artifact
        }
    }
}
