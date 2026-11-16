use super::super::diagnostics::CompactDiagnostics;
use super::super::state::{ArtifactContentMatch, CompactConfirmation, CompactionCandidate};
use super::{
    ArtifactFlow, ArtifactFlowDetector, ArtifactFlowObservation, CandidateDetection,
    CompactDetector, DetectionContext, DetectorMatch,
};

impl CompactDetector for ArtifactFlowDetector {
    fn detect(
        &self,
        candidate: &CompactionCandidate,
        context: &mut DetectionContext<'_, '_>,
    ) -> CandidateDetection {
        let action_id = &candidate.implementation.request.action_id;
        let content = if let Some(cached) = context
            .prepared
            .matches
            .get(action_id)
            .and_then(|pair| pair.content)
        {
            cached
        } else {
            self.detect_content(candidate, context)
        };
        match content {
            ArtifactContentMatch::NoMatch => CandidateDetection::NoMatch,
            ArtifactContentMatch::WorkLimitExceeded => CandidateDetection::WorkLimitExceeded,
            ArtifactContentMatch::Match(flow) => {
                let Some(snapshot) = context.prepared.snapshot.as_ref() else {
                    return CandidateDetection::NoMatch;
                };
                CandidateDetection::Match(DetectorMatch {
                    confirmation: CompactConfirmation::ArtifactFlow {
                        adoption_shape: snapshot.snapshot.shape,
                        flow,
                    },
                    awaiting_trajectory: candidate.implementation.trajectory.is_none(),
                })
            }
        }
    }
}

impl ArtifactFlowDetector {
    pub(in crate::llm_pipeline::projection::compact) fn can_match_candidate(
        &self,
        candidate: &CompactionCandidate,
    ) -> bool {
        // ArtifactFlow candidate admission guarantees a response of sufficient length.
        candidate.implementation.history_source.is_some()
    }

    fn detect_content(
        &self,
        candidate: &CompactionCandidate,
        context: &mut DetectionContext<'_, '_>,
    ) -> ArtifactContentMatch {
        let (Some(snapshot), Some(adoption_source), Some(implementation_history), Some(response)) = (
            context.prepared.snapshot.as_ref(),
            context.prepared.history_source.as_ref(),
            candidate.implementation.history_source.as_ref(),
            candidate.implementation.response_text.as_ref(),
        ) else {
            return ArtifactContentMatch::NoMatch;
        };
        let mut best: Option<(ArtifactFlow, bool)> = None;
        let mut adoption_history = None;
        for observation in self.match_flows(response, snapshot, adoption_source.source.as_ref()) {
            let (flow, structured) = match observation {
                ArtifactFlowObservation::Present(flow) => (flow, false),
                ArtifactFlowObservation::Structured(flow) => (flow, true),
                ArtifactFlowObservation::WorkLimitExceeded => {
                    // The iterator finishes structured proofs before reporting
                    // a text-search limit. Preserve an already accepted proof.
                    if best.is_some() {
                        break;
                    }
                    return ArtifactContentMatch::WorkLimitExceeded;
                }
            };
            if adoption_history.is_none() {
                let histories = context
                    .materializer
                    .histories(implementation_history, adoption_source);
                if let Some(limit) = context.materializer.take_limit_exceeded() {
                    context
                        .diagnostics
                        .push(CompactDiagnostics::limit_diagnostic(
                            context.request,
                            context.request.start_time,
                            limit,
                        ));
                    return ArtifactContentMatch::WorkLimitExceeded;
                }
                let Some([implementation_history, current_history]) = histories else {
                    break;
                };
                if context
                    .history
                    .retains_history(&implementation_history, &current_history)
                {
                    break;
                }
                adoption_history = Some(current_history);
            }
            if adoption_history
                .as_ref()
                .and_then(|history| history.items.get(flow.carrier_item))
                .is_none_or(|carrier| {
                    !matches!(carrier.role.as_str(), "assistant" | "user" | "human")
                })
            {
                continue;
            }
            if best.is_none_or(|(best, best_structured)| {
                flow.bytes > best.bytes
                    || (flow.bytes == best.bytes && flow.carrier_item < best.carrier_item)
                    // Preserve structured evidence priority on an equal carrier
                    // rank while the two paths share one traversal.
                    || (flow.bytes == best.bytes
                        && flow.carrier_item == best.carrier_item
                        && structured && !best_structured)
            }) {
                best = Some((flow, structured));
            }
            if flow.bytes == response.byte_len() {
                break;
            }
        }
        best.map_or(ArtifactContentMatch::NoMatch, |(flow, _)| {
            ArtifactContentMatch::Match(flow)
        })
    }
}
