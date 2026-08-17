use std::collections::VecDeque;
use std::time::SystemTime;

use semantic_action::{LlmTrajectoryStartReason, LlmTrajectoryTransition};

use super::{
    DeferredProviderExpiry, DeferredProviderRequest, INFERENCE_VERSION,
    PendingProviderRegistration, PendingProviderResponse, ProviderBinding, ProviderParent,
    ScopeState, TrajectoryAssignment, TrajectoryClassifierConfig, TrajectoryState,
    singleton_assignment,
};

impl ScopeState {
    pub(super) fn expire_deferred(
        &mut self,
        observed_at: SystemTime,
        config: &TrajectoryClassifierConfig,
    ) -> Vec<TrajectoryAssignment> {
        let mut resolved = Vec::new();
        while let Some(expiry) = self.deferred_expiry.first() {
            if !observed_at
                .duration_since(expiry.observed_at)
                .is_ok_and(|idle| idle >= config.idle_ttl)
            {
                break;
            }
            let Some(expiry) = self.deferred_expiry.pop_first() else {
                break;
            };
            let Some(provider_response_id) = self.deferred_actions.remove(&expiry.action_id) else {
                continue;
            };
            let mut remove_provider_queue = false;
            if let Some(queue) = self.deferred_by_provider.get_mut(&provider_response_id) {
                queue.remove(&expiry.action_id);
                remove_provider_queue = queue.is_empty();
            }
            if remove_provider_queue {
                self.deferred_by_provider.remove(&provider_response_id);
            }
            self.deferred_count = self.deferred_count.saturating_sub(1);
            self.prepare_new_trajectory(observed_at, config);
            let assignment = singleton_assignment(
                expiry.action_id.clone(),
                LlmTrajectoryStartReason::Unspecified,
            );
            self.insert_provider_trajectory(&assignment.trajectory_id, observed_at);
            self.remember_assignment(assignment.clone(), config.assignment_cache_capacity());
            resolved.push(assignment);
            if let Some(pending) = self.pending_provider_responses.remove(&expiry.action_id) {
                resolved.extend(self.resolve_provider_response(
                    &expiry.action_id,
                    &pending.provider_response_id,
                    observed_at,
                    config,
                ));
            }
        }
        resolved
    }

    pub(super) fn defer_provider_request(
        &mut self,
        action_id: String,
        provider_response_id: &str,
        observed_at: SystemTime,
        maximum: usize,
    ) -> bool {
        if self.deferred_count >= maximum || self.deferred_actions.contains_key(&action_id) {
            return false;
        }
        self.deferred_actions
            .insert(action_id.clone(), provider_response_id.to_string());
        self.deferred_expiry.insert(DeferredProviderExpiry {
            observed_at,
            action_id: action_id.clone(),
        });
        self.deferred_by_provider
            .entry(provider_response_id.to_string())
            .or_default()
            .insert(
                action_id.clone(),
                DeferredProviderRequest {
                    action_id,
                    observed_at,
                },
            );
        self.deferred_count += 1;
        true
    }

    pub(super) fn resolve_provider_response(
        &mut self,
        action_id: &str,
        provider_response_id: &str,
        observed_at: SystemTime,
        config: &TrajectoryClassifierConfig,
    ) -> Vec<TrajectoryAssignment> {
        let maximum = config.assignment_cache_capacity();
        if !self.assignments.contains_key(action_id) {
            if self.deferred_actions.contains_key(action_id)
                && self.pending_provider_responses.len() < maximum
            {
                self.pending_provider_responses.insert(
                    action_id.to_string(),
                    PendingProviderResponse {
                        provider_response_id: provider_response_id.to_string(),
                        observed_at,
                    },
                );
            }
            return Vec::new();
        }

        let mut registrations = VecDeque::from([PendingProviderRegistration {
            action_id: action_id.to_string(),
            provider_response_id: provider_response_id.to_string(),
            observed_at,
        }]);
        let mut resolved = Vec::new();
        while let Some(registration) = registrations.pop_front() {
            self.register_provider_response(
                &registration.action_id,
                &registration.provider_response_id,
                registration.observed_at,
                config,
            );
            let Some(waiting) = self
                .deferred_by_provider
                .remove(&registration.provider_response_id)
            else {
                continue;
            };
            for request in waiting.into_values() {
                if self.deferred_actions.remove(&request.action_id).is_none() {
                    continue;
                }
                self.deferred_expiry.remove(&DeferredProviderExpiry {
                    observed_at: request.observed_at,
                    action_id: request.action_id.clone(),
                });
                self.deferred_count = self.deferred_count.saturating_sub(1);
                let assignment = self.classify_provider_reference(
                    request.action_id.clone(),
                    Some(&registration.provider_response_id),
                    request.observed_at.max(registration.observed_at),
                    config,
                );
                self.remember_assignment(assignment.clone(), maximum);
                if let Some(pending) = self.pending_provider_responses.remove(&request.action_id) {
                    registrations.push_back(PendingProviderRegistration {
                        action_id: request.action_id,
                        provider_response_id: pending.provider_response_id,
                        observed_at: pending.observed_at,
                    });
                }
                resolved.push(assignment);
            }
        }
        resolved
    }

    pub(super) fn finalize_deferred(&mut self, maximum: usize) -> Vec<TrajectoryAssignment> {
        let deferred = std::mem::take(&mut self.deferred_by_provider);
        self.deferred_actions.clear();
        self.deferred_expiry.clear();
        self.deferred_count = 0;
        self.pending_provider_responses.clear();
        deferred
            .into_values()
            .flat_map(|requests| requests.into_values())
            .map(|request| {
                let assignment =
                    singleton_assignment(request.action_id, LlmTrajectoryStartReason::Unspecified);
                self.remember_assignment(assignment.clone(), maximum);
                assignment
            })
            .collect()
    }

    pub(super) fn classify_provider_reference(
        &mut self,
        action_id: String,
        previous_response_id: Option<&str>,
        observed_at: SystemTime,
        config: &TrajectoryClassifierConfig,
    ) -> TrajectoryAssignment {
        let parent = previous_response_id.and_then(|provider_response_id| {
            if self.ambiguous_provider_ids.contains(provider_response_id) {
                return None;
            }
            match self.provider_bindings.get(provider_response_id) {
                Some(ProviderBinding::Bound(parent)) => Some(parent.clone()),
                None => None,
            }
        });
        let Some(parent) = parent else {
            self.prepare_new_trajectory(observed_at, config);
            let assignment = singleton_assignment(action_id, LlmTrajectoryStartReason::Unspecified);
            self.insert_provider_trajectory(&assignment.trajectory_id, observed_at);
            return assignment;
        };
        if !self.trajectories.contains_key(&parent.trajectory_id) {
            self.prepare_new_trajectory(observed_at, config);
            let assignment =
                singleton_assignment(action_id, LlmTrajectoryStartReason::CapacityEviction);
            self.insert_provider_trajectory(&assignment.trajectory_id, observed_at);
            return assignment;
        }
        if let Some(trajectory) = self.trajectories.get_mut(&parent.trajectory_id) {
            trajectory.last_seen = trajectory.last_seen.max(observed_at);
        }
        if !self.continued_actions.contains(&parent.action_id) {
            self.continued_actions.insert(parent.action_id.clone());
            if let Some(candidate_id) = self.candidate_by_action.get(&parent.action_id)
                && let Some(candidate) = self.candidates.get_mut(candidate_id)
            {
                candidate.has_continuation = true;
            }
            if let Some(provider_response_id) = previous_response_id
                && let Some(ProviderBinding::Bound(binding)) =
                    self.provider_bindings.get_mut(provider_response_id)
            {
                binding.observed_at = observed_at;
            }
            return TrajectoryAssignment {
                action_id,
                trajectory_id: parent.trajectory_id,
                parent_action_id: Some(parent.action_id),
                forked_from_action_id: None,
                position: parent.position.saturating_add(1),
                transition: LlmTrajectoryTransition::Append,
                start_reason: LlmTrajectoryStartReason::Unspecified,
                inference_version: INFERENCE_VERSION,
            };
        }
        self.prepare_new_trajectory(observed_at, config);
        let assignment = TrajectoryAssignment {
            trajectory_id: action_id.clone(),
            action_id,
            parent_action_id: None,
            forked_from_action_id: Some(parent.action_id),
            position: 0,
            transition: LlmTrajectoryTransition::ForkRoot,
            start_reason: LlmTrajectoryStartReason::Unspecified,
            inference_version: INFERENCE_VERSION,
        };
        self.insert_provider_trajectory(&assignment.trajectory_id, observed_at);
        assignment
    }

    fn insert_provider_trajectory(&mut self, trajectory_id: &str, observed_at: SystemTime) {
        let insertion_order = self.next_candidate_id;
        self.next_candidate_id = self.next_candidate_id.wrapping_add(1);
        self.trajectories
            .entry(trajectory_id.to_string())
            .or_insert_with(|| TrajectoryState {
                candidates: VecDeque::new(),
                provider_response_ids: VecDeque::new(),
                last_seen: observed_at,
                insertion_order,
            });
    }

    fn register_provider_response(
        &mut self,
        action_id: &str,
        provider_response_id: &str,
        observed_at: SystemTime,
        config: &TrajectoryClassifierConfig,
    ) {
        let Some(assignment) = self.assignments.get(action_id).cloned() else {
            return;
        };
        if !self.trajectories.contains_key(&assignment.trajectory_id) {
            return;
        }
        if let Some(ProviderBinding::Bound(parent)) =
            self.provider_bindings.get_mut(provider_response_id)
            && parent.action_id == action_id
        {
            parent.observed_at = observed_at;
            return;
        }
        if self.ambiguous_provider_ids.contains(provider_response_id) {
            return;
        }
        if self.provider_bindings.contains_key(provider_response_id) {
            self.remove_provider_binding(provider_response_id);
            let provider_response_id = provider_response_id.to_string();
            if self
                .ambiguous_provider_ids
                .insert(provider_response_id.clone())
            {
                self.ambiguous_provider_order
                    .push_back(provider_response_id);
            }
            while self.ambiguous_provider_ids.len() > config.assignment_cache_capacity() {
                let Some(expired) = self.ambiguous_provider_order.pop_front() else {
                    break;
                };
                self.ambiguous_provider_ids.remove(&expired);
            }
            return;
        }
        let trajectory_id = assignment.trajectory_id.clone();
        self.provider_bindings.insert(
            provider_response_id.to_string(),
            ProviderBinding::Bound(ProviderParent {
                action_id: action_id.to_string(),
                trajectory_id: trajectory_id.clone(),
                position: assignment.position,
                observed_at,
            }),
        );
        *self
            .provider_binding_counts
            .entry(action_id.to_string())
            .or_default() += 1;
        let expired = {
            let Some(trajectory) = self.trajectories.get_mut(&trajectory_id) else {
                return;
            };
            trajectory.last_seen = trajectory.last_seen.max(observed_at);
            trajectory
                .provider_response_ids
                .push_back(provider_response_id.to_string());
            let mut expired = Vec::new();
            while trajectory
                .candidates
                .len()
                .saturating_add(trajectory.provider_response_ids.len())
                > config.max_candidate_nodes_per_trajectory
            {
                let Some(expired_id) = trajectory.provider_response_ids.pop_front() else {
                    break;
                };
                expired.push(expired_id);
            }
            expired
        };
        for expired_id in expired {
            self.remove_provider_binding(&expired_id);
        }
    }

    pub(super) fn remove_provider_binding(&mut self, provider_response_id: &str) {
        let action_id = match self.provider_bindings.remove(provider_response_id) {
            Some(ProviderBinding::Bound(parent)) => Some(parent.action_id),
            None => None,
        };
        let Some(action_id) = action_id else {
            return;
        };
        let remaining = self
            .provider_binding_counts
            .get_mut(&action_id)
            .map(|count| {
                *count = count.saturating_sub(1);
                *count
            })
            .unwrap_or_default();
        if remaining == 0 {
            self.provider_binding_counts.remove(&action_id);
        }
        if remaining == 0 && !self.candidate_by_action.contains_key(&action_id) {
            self.continued_actions.remove(&action_id);
        }
    }
}
