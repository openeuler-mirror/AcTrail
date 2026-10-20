//! Semantic projection orchestration for normalized action batches.

use std::collections::BTreeMap;

use model_core::diagnostics::LlmPipelineDiagnosticCode;
use model_core::ids::TraceId;
use semantic_action::{SemanticActionKind, attr_keys as attrs};

use crate::live::action_for_live_state;
use crate::llm_pipeline::projection::projector::{
    PendingTrajectoryAction, PendingTrajectoryAdmission, capacity_diagnostic,
};
use crate::llm_pipeline::projection::trajectory::{TrajectoryAssignment, TrajectoryClassification};

use super::super::ProjectionBatch as LiveLlmOutput;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn changed_actions(
        &mut self,
        _config: &config_core::daemon::SemanticRetentionConfig,
        mut output: LiveLlmOutput,
    ) -> LiveLlmOutput {
        let mut changed = LiveLlmOutput::default();
        changed.http1_messages.append(&mut output.http1_messages);
        changed.http1_handled = output.http1_handled;
        changed.http2_frames.append(&mut output.http2_frames);
        changed.http2_handled = output.http2_handled;
        changed.payload_consumed_by_http = output.payload_consumed_by_http;
        changed.payload_consumed_by_llm = output.payload_consumed_by_llm;
        changed.updates.append(&mut output.updates);
        changed.updated_actions.append(&mut output.updated_actions);
        changed.links.append(&mut output.links);
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
        for mut action in output.actions {
            if action.kind == SemanticActionKind::LlmCall {
                // Association owners emit creation and state transitions explicitly.
                changed.actions.push(action);
                continue;
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
            let state_action = action_for_live_state(&action);
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
                if action_record.existing {
                    if action.end_time.is_some()
                        && matches!(
                            action.kind,
                            SemanticActionKind::LlmResponse | SemanticActionKind::WebSocketMessage
                        )
                    {
                        changed
                            .updates
                            .push(crate::live::ActionUpdateFactory::update(
                                &action,
                                semantic_action::SemanticActionChange::ActionSnapshot {
                                    title: action.title.clone(),
                                    attributes: action.attributes.clone(),
                                },
                            ));
                    }
                    changed
                        .updates
                        .push(crate::live::ActionUpdateFactory::lifecycle(&action, None));
                    changed.updated_actions.push(action);
                } else {
                    changed.actions.push(action);
                }
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
        changed.actions.extend(resolved.actions);

        changed.llm_request_contents.extend(resolved.contents);
        changed.llm_request_lineages.extend(resolved.lineages);
        changed.llm_tool_results.extend(resolved.tool_results);
        changed.diagnostics.extend(resolved.diagnostics);
    }
}
