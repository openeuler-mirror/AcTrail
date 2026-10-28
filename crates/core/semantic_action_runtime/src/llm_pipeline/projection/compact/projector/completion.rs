use super::*;

impl ContextCompactProjector {
    pub(in crate::llm_pipeline) fn observe_completed_call(
        &mut self,
        request: &SemanticAction,
        response: CompletedResponse,
        assignments: &[TrajectoryAssignment],
    ) -> CompactProjectionBatch {
        self.observe_completed_call_at(request, response, assignments, SystemTime::now())
    }

    pub(in crate::llm_pipeline::projection::compact) fn observe_completed_call_at(
        &mut self,
        request: &SemanticAction,
        response: CompletedResponse,
        assignments: &[TrajectoryAssignment],
        observed_at: SystemTime,
    ) -> CompactProjectionBatch {
        let mut output = CompactProjectionBatch::empty();
        let affected = self.refine_trajectory_facts(request.trace_id, assignments);
        let trigger = self.admit_completed_call(request, response, observed_at, &mut output);
        self.retry_after_facts(affected, trigger, observed_at, &mut output);
        output.report_work();
        output
    }

    fn admit_completed_call(
        &mut self,
        request: &SemanticAction,
        mut response: CompletedResponse,
        observed_at: SystemTime,
        output: &mut CompactProjectionBatch,
    ) -> Option<CandidateTrigger> {
        let response_start = response.start_time;
        let response_end = response.end_time;
        let Some(key) = CompactScopeKey::from_action(request) else {
            return None;
        };
        let Some(mut state) = self.states.take(&key) else {
            return None;
        };
        let expired = self.expire_scope(&key, &mut state, observed_at, None);
        if expired > 0 {
            output
                .diagnostics
                .push(CompactDiagnostics::entry_diagnostic(
                    request,
                    response_start,
                    LlmPipelineDiagnosticCode::ContextCompactCandidateExpired,
                    expired,
                ));
        }
        let Some(mut pending) = state.take_open(&request.action_id) else {
            self.states.put(key, state);
            return None;
        };
        let valid =
            request.completeness == SemanticActionCompleteness::Complete && response.valid_response;
        if !valid {
            // Adoption is a fact of the complete request, independent of its reply.
            let retain = pending.has_pending_adoption();
            state.put_pending(pending, true, retain);
            if retain {
                self.states.index_pending(&key, &request.action_id);
            } else {
                self.states
                    .remove_pending_index(request.trace_id, &request.action_id);
            }
            self.states.put(key, state);
            return None;
        }
        let should_retain_response = pending.intent.is_none()
            && self.config.inference_enabled
            && !response.has_tool_calls
            && pending.trajectory.as_ref().is_none_or(|fact| {
                fact.allows_inferred_candidate(self.config.inferred_candidate_root_only)
            });
        let response_text = if should_retain_response {
            let content = response.content_text.take();
            match self.flow.response_witness(content) {
                ResponseWitnessObservation::Present(witness) => Some(witness),
                ResponseWitnessObservation::LimitExceeded { bytes } => {
                    output.diagnostics.push(CompactDiagnostics::byte_diagnostic(
                        request,
                        response_end.unwrap_or(response_start),
                        bytes,
                    ));
                    None
                }
                ResponseWitnessObservation::Absent => None,
            }
        } else {
            None
        };
        let detector = DetectorKind::select(pending.intent.as_ref());
        let inferred_candidate = response_text
            .as_ref()
            .is_some_and(|response| self.flow.can_match_response(response));
        let explicit_candidate = pending.intent.is_some();
        let admitted_candidate = inferred_candidate || explicit_candidate;
        let admission_id = admitted_candidate
            .then(|| state.allocate_admission())
            .flatten();
        if admitted_candidate && admission_id.is_none() {
            output
                .diagnostics
                .push(CompactDiagnostics::entry_diagnostic(
                    request,
                    response_end.unwrap_or(response_start),
                    LlmPipelineDiagnosticCode::ContextCompactStateCapacityEvicted,
                    1,
                ));
        }
        let predecessor = Self::record_latest(&mut state, &pending, response_end);
        let mut trigger = None;
        if let Some(admission_id) = admission_id {
            // Optional provenance uses only the existing head at candidate admission.
            // Concurrent completion can make it unsuitable; no older source is searched.
            let predecessor = if explicit_candidate {
                pending.source.take().filter(|source| {
                    source.shape.item_unit == pending.shape.item_unit
                        && source.shape.item_count > pending.shape.item_count
                })
            } else if inferred_candidate && pending.history_source.is_some() {
                predecessor.filter(|source| {
                    source.request.start_time < pending.request.start_time
                        && source
                            .response_end_time
                            .is_some_and(|end| end <= pending.request.start_time)
                        && source.history_source.is_some()
                })
            } else {
                None
            };
            let call = crate::llm_pipeline::projection::llm_call_from_request_response(
                request,
                Some(&response.action_id),
            );
            let head = Self::completed_head(&pending, response, call, response_text);
            let completed_at = head.response_end_time.unwrap_or(head.request.start_time);
            let candidate = CompactionCandidate {
                detector,
                predecessor,
                implementation: head,
                intent: pending.intent.clone(),
                inferred_root_only: self.config.inferred_candidate_root_only,
                expires_at: completed_at
                    .checked_add(self.config.compaction_candidates.ttl)
                    .unwrap_or(completed_at),
            };
            if explicit_candidate
                && let Some(projection) = CompactActionBuilder::prepare(
                    &candidate,
                    None,
                    CompactConfirmation::ExplicitProtocol,
                    candidate.predecessor.as_ref(),
                )
            {
                output.push(projection);
            }
            let capacity = self.config.compaction_candidates.max_entries_per_scope as usize;
            let eviction = (state.candidates.len() >= capacity)
                .then(|| {
                    state.candidates.eviction_candidate(|candidate| {
                        !state.has_waiting_adoption(candidate, observed_at)
                            && !self
                                .states
                                .has_waiting_adoption(&key, candidate, observed_at)
                    })
                })
                .flatten();
            let candidate_trigger = CandidateTrigger::new(key.clone(), admission_id, &candidate);
            let evicted = match state
                .candidates
                .insert(admission_id, candidate, capacity, eviction)
            {
                Ok(evicted) => {
                    trigger = candidate_trigger;
                    evicted
                }
                Err(candidate) => {
                    state.reject_candidate(&candidate, observed_at);
                    self.states.reject_candidate(&key, &candidate, observed_at);
                    1
                }
            };
            if evicted > 0 {
                output
                    .diagnostics
                    .push(CompactDiagnostics::entry_diagnostic(
                        &pending.request,
                        completed_at,
                        LlmPipelineDiagnosticCode::ContextCompactStateCapacityEvicted,
                        evicted,
                    ));
            }
        }
        let retain = state.needs_retention(&pending, true, self.config.trajectory_enabled);
        state.put_pending(pending, true, retain);
        if retain {
            self.states.index_pending(&key, &request.action_id);
        } else {
            self.states
                .remove_pending_index(request.trace_id, &request.action_id);
        }
        self.states.put(key.clone(), state);
        trigger
    }

    fn completed_head(
        pending: &PendingCall,
        response: CompletedResponse,
        call: SemanticAction,
        response_text: Option<ResponseTextWitness>,
    ) -> CompletedCallHead {
        CompletedCallHead {
            request: CompletedRequestHead {
                action_id: pending.request.action_id.clone(),
                start_time: pending.request.start_time,
            },
            response_evidence: response.evidence,
            call,
            shape: pending.shape,
            history_source: pending.history_source.clone(),
            response_end_time: response.end_time,
            trajectory: pending.trajectory.clone(),
            response_text,
        }
    }

    fn record_latest(
        state: &mut CompactScopeState,
        pending: &PendingCall,
        response_end_time: Option<SystemTime>,
    ) -> Option<PredecessorHead> {
        let replace = state.latest_completed.as_ref().is_none_or(|current| {
            (
                current.request.start_time,
                current.request.action_id.as_str(),
            ) <= (
                pending.request.start_time,
                pending.request.action_id.as_str(),
            )
        });
        if replace {
            state.latest_completed.replace(PredecessorHead {
                request: CompletedRequestHead {
                    action_id: pending.request.action_id.clone(),
                    start_time: pending.request.start_time,
                },
                request_evidence: pending.request.evidence.clone(),
                shape: pending.shape,
                history_source: pending.history_source.clone(),
                response_end_time,
            })
        } else {
            None
        }
    }
}
