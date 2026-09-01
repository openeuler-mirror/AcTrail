//! Semantic projection orchestration for normalized action batches.

use std::collections::BTreeMap;

use model_core::diagnostics::LlmPipelineDiagnosticCode;
use model_core::ids::TraceId;
use semantic_action::{SemanticActionKind, SemanticEvidenceKind, attr_keys as attrs};

use crate::live::action_for_live_state;
use crate::llm_pipeline::projection::correlation::{self as call, LlmStreamKey};
use crate::llm_pipeline::projection::projector::{
    self as http, PendingTrajectoryAction, PendingTrajectoryAdmission, capacity_diagnostic,
};
use crate::llm_pipeline::projection::trajectory::{TrajectoryAssignment, TrajectoryClassification};

use super::super::ProjectionBatch as LiveLlmOutput;
use super::super::links::LlmHttpResponseLink;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn changed_actions(
        &mut self,
        config: &config_core::daemon::SemanticRetentionConfig,
        mut output: LiveLlmOutput,
    ) -> LiveLlmOutput {
        let mut changed = LiveLlmOutput::default();
        changed.diagnostics.append(&mut output.diagnostics);
        changed
            .payload_segments
            .append(&mut output.payload_segments);
        changed
            .http_request_links
            .append(&mut output.http_request_links);
        changed
            .http_response_links
            .append(&mut output.http_response_links);
        let non_reusable_response_ids = output.non_reusable_response_ids;
        let mut closed_response_ids = output.closed_response_ids;
        closed_response_ids.extend(non_reusable_response_ids.iter().cloned());
        let mut request_contents = output
            .llm_request_contents
            .into_iter()
            .map(|content| (content.manifest.action_id.clone(), content))
            .collect::<BTreeMap<_, _>>();
        let mut request_histories = output
            .llm_request_histories
            .into_iter()
            .map(|history| (history.action_id.clone(), history))
            .collect::<BTreeMap<_, _>>();
        let mut tool_results = BTreeMap::<String, Vec<_>>::new();
        for result in output.llm_tool_results {
            tool_results
                .entry(result.request_action_id.clone())
                .or_default()
                .push(result);
        }
        let mut provider_response_ids = output
            .provider_response_ids
            .into_iter()
            .map(|metadata| (metadata.action_id, metadata.provider_response_id))
            .collect::<BTreeMap<_, _>>();
        for mut action in output.actions {
            if !config.l4_payload.enabled {
                action
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
            let mut deferred_trajectory = false;
            let mut resolved_trajectories = Vec::new();
            let lineage = if action.kind == SemanticActionKind::LlmRequest {
                self.projector.trajectory.as_mut().and_then(|classifier| {
                    let classification = match request_histories.remove(&action.action_id) {
                        Some(history) => classifier.classify(
                            action.trace_id,
                            action.process,
                            history,
                            action.start_time,
                        ),
                        None => TrajectoryClassification::Assigned(
                            classifier.classify_failure(action.action_id.clone()),
                        ),
                    };
                    resolved_trajectories = classifier.take_resolved();
                    let TrajectoryClassification::Assigned(assignment) = classification else {
                        deferred_trajectory = true;
                        return None;
                    };
                    action.attributes.insert(
                        attrs::llm_request::TRAJECTORY_ID.to_string(),
                        assignment.trajectory_id.clone(),
                    );
                    action.attributes.insert(
                        attrs::llm_request::TRAJECTORY_INFERENCE_VERSION.to_string(),
                        assignment.inference_version.to_string(),
                    );
                    Some(assignment.lineage(action.trace_id))
                })
            } else {
                None
            };
            let mut state_action = action_for_live_state(&action);
            if state_action.kind == SemanticActionKind::LlmResponse {
                if let Some(binding) = self
                    .correlation
                    .active_response_requests
                    .get(&(state_action.trace_id, state_action.action_id.clone()))
                {
                    state_action.attributes.insert(
                        attrs::http_response::REQUEST_ACTION_ID.to_string(),
                        binding.http_request_action_id.clone(),
                    );
                    action.attributes.insert(
                        attrs::http_response::REQUEST_ACTION_ID.to_string(),
                        binding.http_request_action_id.clone(),
                    );
                }
            }
            let response_closed = state_action.kind == SemanticActionKind::LlmResponse
                && closed_response_ids.contains(&state_action.action_id);
            let damaged_http_response = if state_action.kind == SemanticActionKind::LlmResponse {
                let (damaged_http_response, damaged_output) =
                    self.consume_damaged_http_response(&state_action, response_closed);
                changed.extend(damaged_output);
                damaged_http_response
            } else {
                None
            };
            if let Some(http_response) = &damaged_http_response {
                http::mark_response_for_http_failure(&mut state_action, http_response);
                http::mark_response_for_http_failure(&mut action, http_response);
            }
            self.apply_resolved_trajectory_assignments(
                state_action.trace_id,
                resolved_trajectories,
                &mut changed,
            );
            if deferred_trajectory {
                let pending = PendingTrajectoryAction {
                    action: state_action.clone(),
                    content: request_contents.remove(&state_action.action_id),
                    tool_results: tool_results
                        .remove(&state_action.action_id)
                        .unwrap_or_default(),
                };
                match self.projector.defer_trajectory(pending) {
                    PendingTrajectoryAdmission::Deferred => {}
                    PendingTrajectoryAdmission::Evicted(evicted)
                    | PendingTrajectoryAdmission::Rejected(evicted) => {
                        changed.diagnostics.push(capacity_diagnostic(
                            &evicted.action,
                            LlmPipelineDiagnosticCode::PendingTrajectoryCapacityEvicted,
                        ));
                        if let Some(content) = evicted.content {
                            changed.llm_request_contents.push(content);
                        }
                        changed.llm_tool_results.extend(evicted.tool_results);
                        self.push_recorded_action(evicted.action, &mut changed);
                    }
                }
            }
            let action_record = self.projector.record_action(&state_action);
            changed.diagnostics.extend(action_record.diagnostic);
            let action_changed = action_record.changed;
            if action_changed && !deferred_trajectory {
                if let Some(content) = request_contents.remove(&action.action_id) {
                    changed.llm_request_contents.push(content);
                }
                if let Some(lineage) = lineage {
                    changed.llm_request_lineages.push(lineage);
                }
                changed
                    .llm_tool_results
                    .extend(tool_results.remove(&action.action_id).unwrap_or_default());
                changed.actions.push(action);
            }
            match state_action.kind {
                SemanticActionKind::LlmRequest => {
                    changed.extend(self.remember_open_request(state_action.clone()));
                    if let Some(stream_key) = LlmStreamKey::from_llm_request(&state_action) {
                        changed.extend(self.reconcile_exact_websocket_exchange(&stream_key));
                        changed.extend(self.reconcile_confirmed_http_exchanges(&stream_key));
                    }
                }
                SemanticActionKind::LlmResponse => {
                    let provider_response_id =
                        provider_response_ids.remove(&state_action.action_id);
                    if let Some(http_response) = damaged_http_response {
                        changed.http_response_links.push(LlmHttpResponseLink {
                            llm_response: state_action.clone(),
                            http_response,
                        });
                        if !non_reusable_response_ids.contains(&state_action.action_id) {
                            changed.extend(self.remember_pending_response(
                                state_action,
                                provider_response_id,
                                response_closed,
                            ));
                        }
                        continue;
                    }
                    if let Some(binding) = self.request_for_response_update(&state_action) {
                        let assignments = self.projector.register_provider_response(
                            &binding.request,
                            provider_response_id.as_deref(),
                            state_action.end_time.unwrap_or(state_action.start_time),
                        );
                        self.apply_resolved_trajectory_assignments(
                            binding.request.trace_id,
                            assignments,
                            &mut changed,
                        );
                        let mut call = call::llm_call_from_request_response(
                            &binding.request,
                            Some(&state_action),
                        );
                        call.attributes.insert(
                            attrs::llm_call::HTTP_RESPONSE_ACTION_ID.to_string(),
                            binding.http_response_action_id.clone(),
                        );
                        changed.extend(self.update_active_response_request(
                            &state_action,
                            binding,
                            response_closed,
                        ));
                        self.push_recorded_action(call, &mut changed);
                    } else if !non_reusable_response_ids.contains(&state_action.action_id) {
                        changed.extend(self.remember_pending_response(
                            state_action.clone(),
                            provider_response_id,
                            response_closed,
                        ));
                        if let Some(stream_key) = LlmStreamKey::from_llm_response(&state_action) {
                            changed.extend(self.reconcile_exact_websocket_exchange(&stream_key));
                            changed.extend(self.reconcile_confirmed_http_exchanges(&stream_key));
                        }
                    }
                }
                _ => {}
            }
        }
        changed
    }

    pub(in crate::llm_pipeline) fn apply_resolved_trajectory_assignments(
        &mut self,
        trace_id: TraceId,
        assignments: Vec<TrajectoryAssignment>,
        changed: &mut LiveLlmOutput,
    ) {
        let resolved = self.projector.resolve_assignments(trace_id, assignments);
        for request in &resolved.request_updates {
            self.update_open_request(request);
        }
        changed.actions.extend(resolved.actions);
        changed.llm_request_contents.extend(resolved.contents);
        changed.llm_request_lineages.extend(resolved.lineages);
        changed.llm_tool_results.extend(resolved.tool_results);
        changed.diagnostics.extend(resolved.diagnostics);
    }
}
