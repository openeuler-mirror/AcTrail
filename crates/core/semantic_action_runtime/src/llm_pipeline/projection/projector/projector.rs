//! Stateful semantic action projection and trajectory ownership.

use std::collections::BTreeMap;

use config_core::daemon::{LlmRequestContentRetention, SemanticRetentionConfig};
use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use model_core::ids::TraceId;
use std::time::SystemTime;

use semantic_action::{
    LlmRequestContentWrite, LlmRequestLineageWrite, SemanticAction, SemanticActionStatus,
    attr_keys as attrs,
};

use crate::llm_pipeline::projection::trajectory::{TrajectoryAssignment, TrajectoryClassifier};

use super::request::ProjectedLlmToolResult;
use super::state::{BoundedTraceIndex, StateAdmission};

pub(in crate::llm_pipeline) struct PendingTrajectoryAction {
    pub(in crate::llm_pipeline) action: SemanticAction,
    pub(in crate::llm_pipeline) content: Option<LlmRequestContentWrite>,
    pub(in crate::llm_pipeline) tool_results: Vec<ProjectedLlmToolResult>,
}

pub(in crate::llm_pipeline) struct ActionProjector {
    pub(in crate::llm_pipeline) pending_trajectory_actions:
        BTreeMap<(TraceId, String), PendingTrajectoryAction>,
    pub(in crate::llm_pipeline) open_action_versions: BTreeMap<(TraceId, String), SemanticAction>,
    pending_trajectory_index: BoundedTraceIndex,
    action_version_index: BoundedTraceIndex,
    pub(in crate::llm_pipeline) trajectory: Option<TrajectoryClassifier>,
}

pub(in crate::llm_pipeline) struct ActionRecord {
    pub(in crate::llm_pipeline) changed: bool,
    pub(in crate::llm_pipeline) diagnostic: Option<LlmPipelineDiagnostic>,
}

pub(in crate::llm_pipeline) enum PendingTrajectoryAdmission {
    Deferred,
    Evicted(PendingTrajectoryAction),
    Rejected(PendingTrajectoryAction),
}

pub(in crate::llm_pipeline) struct ResolvedTrajectoryBatch {
    pub(in crate::llm_pipeline) actions: Vec<SemanticAction>,
    pub(in crate::llm_pipeline) request_updates: Vec<SemanticAction>,
    pub(in crate::llm_pipeline) contents: Vec<LlmRequestContentWrite>,
    pub(in crate::llm_pipeline) lineages: Vec<LlmRequestLineageWrite>,
    pub(in crate::llm_pipeline) tool_results: Vec<ProjectedLlmToolResult>,
    pub(in crate::llm_pipeline) diagnostics: Vec<LlmPipelineDiagnostic>,
}

impl ActionProjector {
    pub(in crate::llm_pipeline) fn new(config: &SemanticRetentionConfig) -> Self {
        if config.l0_llm_call.enabled
            && config.l0_llm_call.trajectory.enabled
            && !matches!(
                config.l0_llm_call.request_content,
                LlmRequestContentRetention::CanonicalBlocks
            )
        {
            tracing::warn!(
                request_content = ?config.l0_llm_call.request_content,
                "LLM trajectory identification is disabled because request content retention is not canonical_blocks"
            );
        }
        let state = config.l0_llm_call.projection_state;
        Self {
            pending_trajectory_actions: BTreeMap::new(),
            open_action_versions: BTreeMap::new(),
            pending_trajectory_index: BoundedTraceIndex::new(validated_limit(
                state.max_pending_trajectory_actions_per_trace,
            )),
            action_version_index: BoundedTraceIndex::new(validated_limit(
                state.max_action_versions_per_trace,
            )),
            trajectory: config
                .llm_trajectory_enabled()
                .then(|| TrajectoryClassifier::new(config.l0_llm_call.trajectory.into())),
        }
    }

    pub(in crate::llm_pipeline) fn record_action(
        &mut self,
        action: &SemanticAction,
    ) -> ActionRecord {
        let key = (action.trace_id, action.action_id.clone());
        if self.open_action_versions.get(&key) == Some(action) {
            return ActionRecord {
                changed: false,
                diagnostic: None,
            };
        }
        let mut diagnostic = None;
        if action.status == SemanticActionStatus::InProgress {
            if !self.open_action_versions.contains_key(&key) {
                match self.action_version_index.admit(&key) {
                    StateAdmission::Evicted(evicted_key) => {
                        if let Some(evicted) = self.open_action_versions.remove(&evicted_key) {
                            diagnostic = Some(capacity_diagnostic(
                                &evicted,
                                LlmPipelineDiagnosticCode::ActionVersionCapacityEvicted,
                            ));
                        }
                    }
                    StateAdmission::SequenceExhausted => {
                        return ActionRecord {
                            changed: true,
                            diagnostic: Some(capacity_diagnostic(
                                action,
                                LlmPipelineDiagnosticCode::ActionVersionSequenceExhausted,
                            )),
                        };
                    }
                    StateAdmission::Inserted | StateAdmission::Existing => {}
                }
            }
            self.open_action_versions.insert(key, action.clone());
        } else {
            self.open_action_versions.remove(&key);
            self.action_version_index.remove(&key);
        }
        ActionRecord {
            changed: true,
            diagnostic,
        }
    }

    pub(in crate::llm_pipeline) fn defer_trajectory(
        &mut self,
        pending: PendingTrajectoryAction,
    ) -> PendingTrajectoryAdmission {
        let key = (pending.action.trace_id, pending.action.action_id.clone());
        match self.pending_trajectory_index.admit(&key) {
            StateAdmission::Existing => {
                self.pending_trajectory_actions.insert(key, pending);
                PendingTrajectoryAdmission::Deferred
            }
            StateAdmission::Inserted => {
                self.pending_trajectory_actions.insert(key, pending);
                PendingTrajectoryAdmission::Deferred
            }
            StateAdmission::Evicted(evicted_key) => {
                let evicted = self.pending_trajectory_actions.remove(&evicted_key);
                self.pending_trajectory_actions.insert(key, pending);
                evicted.map_or(
                    PendingTrajectoryAdmission::Deferred,
                    PendingTrajectoryAdmission::Evicted,
                )
            }
            StateAdmission::SequenceExhausted => PendingTrajectoryAdmission::Rejected(pending),
        }
    }

    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        for key in self.pending_trajectory_index.forget_trace(trace_id) {
            self.pending_trajectory_actions.remove(&key);
        }
        for key in self.action_version_index.forget_trace(trace_id) {
            self.open_action_versions.remove(&key);
        }
    }

    pub(in crate::llm_pipeline) fn register_provider_response(
        &mut self,
        request: &SemanticAction,
        provider_response_id: Option<&str>,
        observed_at: SystemTime,
    ) -> Vec<TrajectoryAssignment> {
        let Some(provider_response_id) = provider_response_id else {
            return Vec::new();
        };
        let Some(classifier_id) = request.attributes.get(attrs::llm_request::CLASSIFIER_ID) else {
            return Vec::new();
        };
        self.trajectory
            .as_mut()
            .map_or_else(Vec::new, |classifier| {
                classifier.register_provider_response(
                    request.trace_id,
                    request.process.clone(),
                    classifier_id.clone(),
                    &request.action_id,
                    provider_response_id,
                    observed_at,
                )
            })
    }

    pub(in crate::llm_pipeline) fn reject_trajectory_parent(&mut self, request: &SemanticAction) {
        let Some(classifier_id) = request.attributes.get(attrs::llm_request::CLASSIFIER_ID) else {
            return;
        };
        if let Some(classifier) = self.trajectory.as_mut() {
            classifier.reject_parent_candidate(
                request.trace_id,
                request.process,
                classifier_id,
                &request.action_id,
            );
        }
    }

    pub(in crate::llm_pipeline) fn resolve_assignments(
        &mut self,
        trace_id: TraceId,
        assignments: Vec<TrajectoryAssignment>,
    ) -> ResolvedTrajectoryBatch {
        let mut batch = ResolvedTrajectoryBatch {
            actions: Vec::new(),
            request_updates: Vec::new(),
            contents: Vec::new(),
            lineages: Vec::new(),
            tool_results: Vec::new(),
            diagnostics: Vec::new(),
        };
        for assignment in assignments {
            let Some(pending) = self
                .pending_trajectory_actions
                .remove(&(trace_id, assignment.action_id.clone()))
            else {
                continue;
            };
            self.pending_trajectory_index
                .remove(&(trace_id, assignment.action_id.clone()));
            let mut action = pending.action;
            action.attributes.insert(
                attrs::llm_request::TRAJECTORY_ID.to_string(),
                assignment.trajectory_id.clone(),
            );
            action.attributes.insert(
                attrs::llm_request::TRAJECTORY_INFERENCE_VERSION.to_string(),
                assignment.inference_version.to_string(),
            );
            batch.lineages.push(assignment.lineage(action.trace_id));
            if let Some(content) = pending.content {
                batch.contents.push(content);
            }
            batch.tool_results.extend(pending.tool_results);
            batch.request_updates.push(action.clone());
            let record = self.record_action(&action);
            batch.diagnostics.extend(record.diagnostic);
            if record.changed {
                batch.actions.push(action);
            }
        }
        batch
    }
}

fn validated_limit(value: u32) -> usize {
    usize::try_from(value).expect("validated LLM projection-state limit must fit usize")
}

pub(in crate::llm_pipeline) fn capacity_diagnostic(
    action: &SemanticAction,
    code: LlmPipelineDiagnosticCode,
) -> LlmPipelineDiagnostic {
    let mut stream_key = action
        .attributes
        .get(attrs::payload::STREAM_KEY)
        .cloned()
        .unwrap_or_else(|| format!("diagnostic:{}", code.as_u16()));
    if let Some(stream_id) = action
        .attributes
        .get(attrs::http_request::STREAM_ID)
        .or_else(|| action.attributes.get(attrs::http_response::STREAM_ID))
    {
        stream_key.push_str("#h2:");
        stream_key.push_str(stream_id);
    }
    LlmPipelineDiagnostic::new(
        action.trace_id,
        &action.process,
        action.end_time.unwrap_or(action.start_time),
        code,
        LlmPipelineDiagnosticSeverity::Warning,
        LlmPipelineDiagnosticStage::Correlation,
    )
    .with_stream_key(&stream_key)
    .with_discarded_entries(1)
}
