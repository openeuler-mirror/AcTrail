use std::sync::Arc;
use std::time::SystemTime;

use semantic_action::SemanticAction;

use super::{CompactDiagnostics, ContextCompactProjector, LlmRequestObservation};
use crate::llm_pipeline::projection::compact::builder::{
    CompactProjectionBatch, ConfirmedAdoption,
};
use crate::llm_pipeline::projection::compact::detector::{
    CarrierSnapshotObservation, DetectionContext,
};
use crate::llm_pipeline::projection::compact::gate::TrajectoryFact;
use crate::llm_pipeline::projection::compact::history::HistoryMaterializer;
use crate::llm_pipeline::projection::compact::state::{
    AdmissionId, ArtifactContentMatch, CandidateDetection, CandidateSource, CompactConfirmation,
    CompactScopeKey, CompactScopeState, CompactionCandidate, DetectionOutcome, DetectorKind,
    PairEvidence, PairStatus, PendingAdoption, PreparedCarrierSnapshot, RequestHistoryWitness,
};
use model_core::diagnostics::LlmPipelineDiagnosticCode;

impl ContextCompactProjector {
    pub(super) fn prepare_adoption(
        &self,
        current: Option<&TrajectoryFact>,
        observation: &LlmRequestObservation,
        history_source: Option<Arc<RequestHistoryWitness>>,
        request: &SemanticAction,
        output: &mut CompactProjectionBatch,
    ) -> Option<PendingAdoption> {
        if current.is_some_and(|fact| !fact.allows_adoption()) {
            return None;
        }
        let mut prepared = PendingAdoption {
            snapshot: None,
            snapshot_prepared: false,
            matches: Default::default(),
            initialized: false,
            choices: Default::default(),
            opaque: None,
            history_source,
            expires_at: request
                .start_time
                .checked_add(self.config.pending_adoptions.ttl)
                .unwrap_or(request.start_time),
        };
        if let Some(limit) = self.protocol.prepare_adoption(
            &observation.context,
            observation.history.as_ref(),
            &mut prepared,
        ) {
            output
                .diagnostics
                .push(CompactDiagnostics::limit_diagnostic(
                    request,
                    request.start_time,
                    limit,
                ));
        }
        (prepared.opaque.is_some()
            || (self.config.trajectory_enabled && prepared.history_source.is_some()))
        .then_some(prepared)
    }

    /// Recall the initial candidate collection once. Subsequent B changes update one pair.
    pub(super) fn initialize_adoption(
        &self,
        state: &CompactScopeState,
        scope: &CompactScopeKey,
        request: &SemanticAction,
        current: Option<&TrajectoryFact>,
        prepared: &mut PendingAdoption,
        output: &mut CompactProjectionBatch,
    ) {
        if prepared.initialized || (current.is_none() && self.config.trajectory_enabled) {
            return;
        }
        output.work.initializations = output.work.initializations.saturating_add(1);
        for (id, candidate) in state
            .candidates
            .for_adoption(request.start_time, self.config.compaction_candidates.ttl)
        {
            self.update_pair(scope, id, candidate, request, current, prepared, output);
        }
        for (owner, id, candidate) in self.states.process_candidates(
            scope,
            request.start_time,
            self.config.compaction_candidates.ttl,
        ) {
            if candidate.detector == DetectorKind::ArtifactFlow {
                self.update_pair(owner, id, candidate, request, current, prepared, output);
            }
        }
        prepared.initialized = true;
    }

    pub(super) fn update_pair(
        &self,
        owner: &CompactScopeKey,
        candidate_id: AdmissionId,
        candidate: &CompactionCandidate,
        request: &SemanticAction,
        current: Option<&TrajectoryFact>,
        prepared: &mut PendingAdoption,
        output: &mut CompactProjectionBatch,
    ) -> bool {
        let kind = candidate.detector;
        if !prepared.supports(kind) {
            return false;
        }
        output.work.pair_checks = output.work.pair_checks.saturating_add(1);
        let action_id = &candidate.implementation.request.action_id;
        let previous = prepared
            .matches
            .get(action_id)
            .map_or(PairStatus::NoMatch, |pair| pair.status);
        let applicable = candidate.applies_to(request, current, &self.gate)
            && (kind != DetectorKind::ArtifactFlow
                || (current.is_some() && self.flow.can_match_candidate(candidate)));
        if !applicable {
            if let Some(pair) = prepared.matches.get_mut(action_id) {
                pair.status = PairStatus::NoMatch;
                prepared.choices.group(kind).update(
                    previous,
                    PairStatus::NoMatch,
                    CandidateSource {
                        owner: owner.clone(),
                        admission_id: candidate_id,
                        action_id: action_id.clone(),
                    },
                );
            }
            return previous != PairStatus::NoMatch;
        }
        if kind == DetectorKind::ArtifactFlow && !self.prepare_carriers(prepared, request, output) {
            return false;
        }
        prepared
            .matches
            .entry(action_id.clone())
            .or_insert_with(|| PairEvidence::new(kind));
        if kind == DetectorKind::ArtifactFlow
            && prepared
                .matches
                .get(action_id)
                .is_some_and(|pair| pair.content.is_none())
        {
            output.work.content_checks = output.work.content_checks.saturating_add(1);
        }
        let mut materializer = HistoryMaterializer::new(&self.history);
        let mut context = DetectionContext {
            history: &self.history,
            materializer: &mut materializer,
            allow_protocol_without_trajectory: !self.config.trajectory_enabled,
            request,
            current,
            prepared,
            diagnostics: &mut output.diagnostics,
        };
        let (status, content) = match kind
            .detector(&self.protocol, &self.flow)
            .detect(candidate, &mut context)
        {
            CandidateDetection::NoMatch => {
                (PairStatus::NoMatch, Some(ArtifactContentMatch::NoMatch))
            }
            CandidateDetection::Match(detected) => (
                if detected.awaiting_trajectory {
                    PairStatus::Pending
                } else {
                    PairStatus::Ready
                },
                match detected.confirmation {
                    CompactConfirmation::ArtifactFlow { flow, .. } => {
                        Some(ArtifactContentMatch::Match(flow))
                    }
                    _ => None,
                },
            ),
            CandidateDetection::WorkLimitExceeded => {
                if previous != PairStatus::Limited {
                    context
                        .diagnostics
                        .push(CompactDiagnostics::entry_diagnostic(
                            request,
                            request.start_time,
                            LlmPipelineDiagnosticCode::ContextCompactMatchWorkLimitExceeded,
                            1,
                        ));
                }
                (
                    PairStatus::Limited,
                    Some(ArtifactContentMatch::WorkLimitExceeded),
                )
            }
        };
        if let Some(pair) = context.prepared.matches.get_mut(action_id) {
            pair.status = status;
            pair.content = content;
        }
        context.prepared.choices.group(kind).update(
            previous,
            status,
            CandidateSource {
                owner: owner.clone(),
                admission_id: candidate_id,
                action_id: action_id.clone(),
            },
        );
        previous != status
    }

    fn prepare_carriers(
        &self,
        prepared: &mut PendingAdoption,
        request: &SemanticAction,
        output: &mut CompactProjectionBatch,
    ) -> bool {
        if !prepared.snapshot_prepared {
            prepared.snapshot_prepared = true;
            if let Some(source) = prepared.history_source.as_ref() {
                match self.flow.carrier_snapshot(source.source.as_ref()) {
                    CarrierSnapshotObservation::Present(snapshot) => {
                        prepared.snapshot = Some(PreparedCarrierSnapshot::new(snapshot))
                    }
                    CarrierSnapshotObservation::LimitExceeded(limit) => {
                        output
                            .diagnostics
                            .push(CompactDiagnostics::limit_diagnostic(
                                request,
                                request.start_time,
                                limit,
                            ))
                    }
                    CarrierSnapshotObservation::Absent => {}
                }
            }
        }
        prepared.snapshot.is_some()
    }

    pub(super) fn confirm_adoption(
        &mut self,
        state: &mut CompactScopeState,
        scope: &CompactScopeKey,
        request: &SemanticAction,
        current: Option<&TrajectoryFact>,
        prepared: Option<&mut PendingAdoption>,
        observed_at: SystemTime,
        output: &mut CompactProjectionBatch,
    ) -> DetectionOutcome {
        let Some(prepared) = prepared.filter(|adoption| adoption.can_wait(observed_at)) else {
            return DetectionOutcome::Unconfirmed;
        };
        let choice = prepared.choices.selected();
        if let Some(count) = choice.take_ambiguity() {
            output
                .diagnostics
                .push(CompactDiagnostics::entry_diagnostic(
                    request,
                    request.start_time,
                    LlmPipelineDiagnosticCode::ContextCompactSequenceAmbiguous,
                    count,
                ));
        }
        let Some(source) = choice.unique().cloned() else {
            return DetectionOutcome::Unconfirmed;
        };
        let Some(confirmation) = prepared.confirmation(&source.action_id) else {
            return DetectionOutcome::Unconfirmed;
        };
        let candidate = if source.owner == *scope {
            state.candidates.get(source.admission_id)
        } else {
            self.states.candidate(&source.owner, source.admission_id)
        };
        let Some(candidate) = candidate
            .filter(|candidate| candidate.implementation.request.action_id == source.action_id)
        else {
            prepared.choices.selected().discard(PairStatus::Ready);
            return DetectionOutcome::Unconfirmed;
        };
        let mut materializer = HistoryMaterializer::new(&self.history);
        let predecessor = candidate.predecessor.as_ref().filter(|previous| {
            if candidate.detector == DetectorKind::ArtifactFlow {
                match (
                    previous.history_source.as_ref(),
                    candidate.implementation.history_source.as_ref(),
                ) {
                    (Some(previous), Some(implementation)) => {
                        materializer.consumes_source(previous, implementation)
                    }
                    _ => false,
                }
            } else {
                true
            }
        });
        if let Some(limit) = materializer.take_limit_exceeded() {
            output
                .diagnostics
                .push(CompactDiagnostics::limit_diagnostic(
                    request,
                    request.start_time,
                    limit,
                ));
        }
        let Some(projection) = super::CompactActionBuilder::prepare(
            candidate,
            Some(request),
            confirmation,
            predecessor,
        ) else {
            return DetectionOutcome::Unconfirmed;
        };
        let consumed = if source.owner == *scope {
            state.candidates.remove(source.admission_id)
        } else {
            self.states
                .remove_candidate(&source.owner, source.admission_id)
        };
        let Some(consumed) = consumed else {
            return DetectionOutcome::Unconfirmed;
        };
        for action_id in
            state.retire_candidate(&super::super::state::RetiredCandidate::from(&consumed))
        {
            self.states.mark_dirty(scope, action_id);
        }
        output.push(projection);
        if current.is_some_and(TrajectoryFact::allows_adoption) {
            output
                .confirmed_adoptions
                .push(ConfirmedAdoption::new(scope, &request.action_id));
        }
        DetectionOutcome::Confirmed
    }
}
