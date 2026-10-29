use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{LlmContextCompactConfig, LlmContextCompactPoolConfig};
use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, attr_keys as attrs,
};

use super::builder::{CompactActionBuilder, CompactProjectionBatch};
use super::detector::{ArtifactFlowDetector, ExplicitProtocolDetector};
use super::diagnostics::CompactDiagnostics;
use super::gate::{TrajectoryBoundaryGate, TrajectoryFact, TrajectorySlot};
use super::history::HistoryContinuationFilter;
use super::observation::{CompletedResponse, LlmRequestObservation};
use super::state::{
    CompactConfirmation, CompactScopeKey, CompactScopeState, CompactStateStore,
    CompactionCandidate, CompletedCallHead, CompletedRequestHead, DetectionOutcome, DetectorKind,
    PendingCall, PredecessorHead, RequestHistoryWitness, ResponseTextWitness,
    ResponseWitnessObservation,
};
use crate::llm_pipeline::projection::trajectory::TrajectoryAssignment;
use std::sync::Arc;

mod completion;
mod confirmation;
mod retry;
use retry::CandidateTrigger;

pub(in crate::llm_pipeline) struct ContextCompactProjector {
    config: RuntimeConfig,
    gate: TrajectoryBoundaryGate,
    protocol: ExplicitProtocolDetector,
    flow: ArtifactFlowDetector,
    history: HistoryContinuationFilter,
    states: CompactStateStore,
}

#[derive(Clone, Copy)]
struct RuntimeConfig {
    inference_enabled: bool,
    inferred_candidate_root_only: bool,
    trajectory_enabled: bool,
    max_open_requests_per_scope: usize,
    compaction_candidates: LlmContextCompactPoolConfig,
    pending_adoptions: LlmContextCompactPoolConfig,
    max_history_source_bytes: usize,
}

impl ContextCompactProjector {
    pub(in crate::llm_pipeline) fn new(
        config: LlmContextCompactConfig,
        trajectory_enabled: bool,
    ) -> Self {
        let max_artifact_bytes = usize::try_from(config.max_artifact_bytes)
            .expect("validated context compaction artifact bytes must fit usize");
        let max_context_scan_bytes = usize::try_from(config.max_context_scan_bytes)
            .expect("validated context compaction scan bytes must fit usize");
        let max_history_source_bytes = usize::try_from(config.max_history_source_bytes)
            .expect("validated compact history source bytes must fit usize");
        let max_history_snapshot_bytes = max_history_source_bytes
            .checked_mul(2)
            .expect("validated compact history snapshot bytes must fit usize");
        let max_context_items = config.max_context_items_to_scan as usize;
        let max_fingerprint_positions = config.max_fingerprint_positions as usize;
        Self {
            config: RuntimeConfig {
                inference_enabled: config.inference_enabled,
                inferred_candidate_root_only: config.inferred_candidate_root_only,
                trajectory_enabled,
                max_open_requests_per_scope: config.max_open_requests_per_scope as usize,
                compaction_candidates: config.compaction_candidates,
                pending_adoptions: config.pending_adoptions,
                max_history_source_bytes,
            },
            gate: TrajectoryBoundaryGate,
            protocol: ExplicitProtocolDetector::new(max_context_items, max_artifact_bytes),
            flow: ArtifactFlowDetector::new(
                max_artifact_bytes,
                max_context_items,
                max_context_scan_bytes,
                config.min_artifact_flow_bytes as usize,
                usize::from(config.min_artifact_coverage_percent),
                max_fingerprint_positions,
            ),
            states: CompactStateStore::new(config.max_active_scopes_per_trace as usize),
            history: HistoryContinuationFilter::new(max_context_items, max_history_snapshot_bytes),
        }
    }

    pub(in crate::llm_pipeline) fn observe_request(
        &mut self,
        request: &SemanticAction,
        observation: Option<LlmRequestObservation>,
        trajectory: Option<&TrajectoryAssignment>,
    ) -> CompactProjectionBatch {
        self.observe_request_at(request, observation, trajectory, SystemTime::now())
    }

    pub(super) fn observe_request_at(
        &mut self,
        request: &SemanticAction,
        observation: Option<LlmRequestObservation>,
        trajectory: Option<&TrajectoryAssignment>,
        observed_at: SystemTime,
    ) -> CompactProjectionBatch {
        let mut output = CompactProjectionBatch::empty();
        if request.status != SemanticActionStatus::Success
            || request.completeness != SemanticActionCompleteness::Complete
            || matches!(
                request
                    .attributes
                    .get(attrs::llm_request::BACKGROUND_KIND)
                    .map(String::as_str),
                Some("title_generation")
            )
        {
            return output;
        }
        let Some(observation) = observation.filter(|value| value.action_id == request.action_id)
        else {
            return output;
        };
        let Some(shape) = self.flow.request_shape(&observation.context) else {
            return output;
        };
        let Some(key) = CompactScopeKey::from_action(request) else {
            return output;
        };
        let admission = self.states.admit(&key);
        if let Some((evicted, entries)) = admission.evicted
            && entries > 0
        {
            output.diagnostics.push(
                LlmPipelineDiagnostic::new(
                    evicted.trace_id,
                    &evicted.process,
                    request.start_time,
                    LlmPipelineDiagnosticCode::ContextCompactStateCapacityEvicted,
                    LlmPipelineDiagnosticSeverity::Warning,
                    LlmPipelineDiagnosticStage::Correlation,
                )
                .with_discarded_entries(u64::try_from(entries).unwrap_or(u64::MAX)),
            );
        }
        let intent = self.protocol.request_intent(&observation.context);
        let fact = trajectory.map(TrajectoryFact::from);
        let mut state = self.states.take(&key).unwrap_or_default();
        let expired = self.expire_scope(
            &key,
            &mut state,
            observed_at,
            Some((request, fact.as_ref())),
        );
        if expired > 0 {
            output
                .diagnostics
                .push(CompactDiagnostics::entry_diagnostic(
                    request,
                    request.start_time,
                    LlmPipelineDiagnosticCode::ContextCompactCandidateExpired,
                    expired,
                ));
        }
        if state.open_calls.contains_key(&request.action_id)
            || state.retained_calls.contains_key(&request.action_id)
        {
            self.states.put(key, state);
            return output;
        }
        let Some(admission_id) = state.allocate_admission() else {
            output
                .diagnostics
                .push(CompactDiagnostics::entry_diagnostic(
                    request,
                    request.start_time,
                    LlmPipelineDiagnosticCode::ContextCompactStateCapacityEvicted,
                    1,
                ));
            self.states.put(key, state);
            return output;
        };
        while state.open_calls.len() >= self.config.max_open_requests_per_scope {
            let Some(evicted_id) = state
                .pending_order
                .iter()
                .find(|id| state.open_calls.contains_key(*id))
                .cloned()
            else {
                break;
            };
            let Some(pending) = state.take_open(&evicted_id) else {
                break;
            };
            let retain = pending.has_pending_adoption();
            state.put_pending(pending, true, retain);
            if !retain {
                self.states
                    .remove_pending_index(request.trace_id, &evicted_id);
            }
            output
                .diagnostics
                .push(CompactDiagnostics::entry_diagnostic(
                    request,
                    request.start_time,
                    LlmPipelineDiagnosticCode::ContextCompactStateCapacityEvicted,
                    1,
                ));
        }
        let source = intent.as_ref().and_then(|_| {
            state
                .latest_completed
                .as_ref()
                .filter(|source| {
                    source
                        .response_end_time
                        .is_some_and(|end| end <= request.start_time)
                })
                .cloned()
        });
        let history_source = if !self.config.inference_enabled {
            None
        } else if observation.history_source_bytes > self.config.max_history_source_bytes {
            output.diagnostics.push(CompactDiagnostics::byte_diagnostic(
                request,
                request.start_time,
                observation.history_source_bytes,
            ));
            None
        } else {
            Some(Arc::new(RequestHistoryWitness::new(Arc::clone(
                &observation.history,
            ))))
        };
        let mut pending_adoption = self.prepare_adoption(
            fact.as_ref(),
            &observation,
            history_source.clone(),
            request,
            &mut output,
        );
        if let Some(adoption) = pending_adoption.as_mut() {
            self.initialize_adoption(&state, &key, request, fact.as_ref(), adoption, &mut output);
        }
        let confirmed_current = self.confirm_adoption(
            &mut state,
            &key,
            request,
            fact.as_ref(),
            pending_adoption.as_mut(),
            observed_at,
            &mut output,
        );
        if matches!(confirmed_current, DetectionOutcome::Confirmed) {
            pending_adoption = None;
        }
        if pending_adoption
            .as_ref()
            .is_some_and(|adoption| adoption.can_wait(observed_at))
        {
            let evicted = state
                .make_adoption_room(self.config.pending_adoptions.max_entries_per_scope as usize);
            for action_id in &evicted {
                if state.open_calls.get(action_id).is_some_and(|pending| {
                    !state.needs_retention(pending, false, self.config.trajectory_enabled)
                }) {
                    self.states
                        .remove_pending_index(request.trace_id, action_id);
                }
            }
            if !evicted.is_empty() {
                output
                    .diagnostics
                    .push(CompactDiagnostics::entry_diagnostic(
                        request,
                        request.start_time,
                        LlmPipelineDiagnosticCode::ContextCompactStateCapacityEvicted,
                        evicted.len(),
                    ));
            }
        }
        let pending = PendingCall {
            admission_id,
            request: PendingCall::request_metadata(request, &key.classifier_id),
            shape,
            history_source,
            intent,
            trajectory: TrajectorySlot::new(fact),
            source,
            adoption: pending_adoption.filter(|adoption| adoption.can_wait(observed_at)),
        };
        let retain_index = state.needs_retention(&pending, false, self.config.trajectory_enabled);
        state.insert_request(pending);
        self.states.put(key.clone(), state);
        if retain_index {
            self.states.index_pending(&key, &request.action_id);
        }
        self.commit_adoptions(observed_at, &mut output);
        output.report_work();
        output
    }

    pub(in crate::llm_pipeline) fn resolve_trajectory_assignments(
        &mut self,
        trace_id: TraceId,
        assignments: &[TrajectoryAssignment],
    ) -> CompactProjectionBatch {
        self.resolve_trajectory_assignments_at(trace_id, assignments, SystemTime::now())
    }

    pub(super) fn resolve_trajectory_assignments_at(
        &mut self,
        trace_id: TraceId,
        assignments: &[TrajectoryAssignment],
        observed_at: SystemTime,
    ) -> CompactProjectionBatch {
        let mut output = CompactProjectionBatch::empty();
        let affected = self.refine_trajectory_facts(trace_id, assignments);
        self.retry_after_facts(affected, None, observed_at, &mut output);
        output.report_work();
        output
    }

    fn refine_trajectory_facts(
        &mut self,
        trace_id: TraceId,
        assignments: &[TrajectoryAssignment],
    ) -> Vec<(CompactScopeKey, String)> {
        let mut affected = Vec::new();
        for assignment in assignments {
            let Some(key) = self.states.pending_scope(trace_id, &assignment.action_id) else {
                continue;
            };
            if self.states.refine_trajectory(&key, assignment) {
                affected.push((key.clone(), assignment.action_id.clone()));
            }
        }
        affected
    }

    fn retry_after_facts(
        &mut self,
        affected: Vec<(CompactScopeKey, String)>,
        trigger: Option<CandidateTrigger>,
        observed_at: SystemTime,
        output: &mut CompactProjectionBatch,
    ) {
        let mut triggers = BTreeMap::new();
        if let Some(trigger) = trigger {
            triggers.insert((trigger.scope.clone(), trigger.admission_id), trigger);
        }
        let mut requests = BTreeMap::<_, Vec<_>>::new();
        for (key, action_id) in affected {
            if let Some((id, candidate)) = self.states.candidate_for_action(&key, &action_id)
                && let Some(trigger) = CandidateTrigger::new(key.clone(), id, candidate)
            {
                triggers.insert((key.clone(), id), trigger);
            }
            requests.entry(key).or_default().push(action_id);
        }
        for (key, ids) in requests {
            self.initialize_pending(&key, &ids, observed_at, output);
        }
        for trigger in triggers.into_values() {
            self.retry_process(&trigger, observed_at, output);
        }
        self.commit_adoptions(observed_at, output);
    }

    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        self.states.forget_trace(trace_id);
    }

    pub(in crate::llm_pipeline) fn forget_pending_request(
        &mut self,
        trace_id: TraceId,
        action_id: &str,
    ) {
        let Some(key) = self.states.pending_scope(trace_id, action_id) else {
            return;
        };
        let Some(mut state) = self.states.take(&key) else {
            return;
        };
        state.take_pending(action_id);
        state
            .pending_order
            .retain(|candidate| candidate != action_id);
        state.candidates.retain(|candidate| {
            if candidate
                .predecessor
                .as_ref()
                .is_some_and(|predecessor| predecessor.request.action_id == action_id)
            {
                if candidate.intent.is_some() {
                    return false;
                }
                candidate.predecessor = None;
            }
            candidate.implementation.request.action_id != action_id
        });
        if state
            .latest_completed
            .as_ref()
            .is_some_and(|latest| latest.request.action_id == action_id)
        {
            state.latest_completed = None;
        }
        self.states.remove_pending_index(trace_id, action_id);
        self.states.put(key, state);
    }
}

impl CompactScopeKey {
    fn from_action(action: &SemanticAction) -> Option<Self> {
        Some(Self {
            trace_id: action.trace_id,
            process: action.process,
            classifier_id: action
                .attributes
                .get(attrs::llm_request::CLASSIFIER_ID)?
                .clone(),
        })
    }
}
