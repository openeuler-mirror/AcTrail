use crate::probe_detector::contract::candidate::ProbeCandidate;
use crate::probe_detector::contract::detection::{
    AmbiguousDetection, DetectionEvidence, DetectionOutcome,
};

use super::SelectionPolicy;

pub(crate) struct DetectionSelector {
    policy: SelectionPolicy,
}

impl DetectionSelector {
    pub(crate) fn new(policy: SelectionPolicy) -> Self {
        Self { policy }
    }

    pub(crate) fn select(
        &self,
        mut evidence: DetectionEvidence,
        outcomes: Vec<DetectionOutcome>,
    ) -> DetectionOutcome {
        evidence.children = outcomes;
        match self.policy {
            SelectionPolicy::FirstComplete => Self::first_complete(evidence),
            SelectionPolicy::UniqueMatch => Self::unique_match(evidence),
            SelectionPolicy::UniqueClosure => Self::unique_closure(evidence),
            SelectionPolicy::CollectAll => DetectionOutcome::Collected(evidence),
            SelectionPolicy::SelectApplicable => Self::select_applicable(evidence),
        }
    }

    pub(crate) fn should_stop(&self, outcome: &DetectionOutcome) -> bool {
        if self.policy != SelectionPolicy::FirstComplete {
            return false;
        }
        matches!(
            outcome,
            DetectionOutcome::Matched(candidate)
                if candidate.capability.executable_by_consumer()
        ) || matches!(
            outcome,
            DetectionOutcome::Ambiguous(ambiguous)
                if Self::ambiguous_executable_by_consumer(ambiguous)
        )
    }

    fn first_complete(evidence: DetectionEvidence) -> DetectionOutcome {
        for outcome in &evidence.children {
            match outcome {
                DetectionOutcome::Matched(candidate)
                    if candidate.capability.executable_by_consumer() =>
                {
                    return Self::selected(candidate.clone(), &evidence);
                }
                DetectionOutcome::Ambiguous(ambiguous)
                    if Self::ambiguous_executable_by_consumer(ambiguous) =>
                {
                    return DetectionOutcome::Ambiguous(AmbiguousDetection {
                        candidates: ambiguous.candidates.clone(),
                        evidence,
                    });
                }
                _ => {}
            }
        }
        if Self::all_inapplicable(&evidence) {
            DetectionOutcome::Inapplicable(evidence)
        } else {
            DetectionOutcome::NoMatch(evidence)
        }
    }

    fn unique_match(evidence: DetectionEvidence) -> DetectionOutcome {
        if let Some(ambiguous) = Self::first_ambiguous(&evidence) {
            return DetectionOutcome::Ambiguous(AmbiguousDetection {
                candidates: ambiguous.candidates,
                evidence,
            });
        }
        let matched = Self::matched(&evidence);
        Self::unique(matched, evidence)
    }

    fn unique(mut matched: Vec<ProbeCandidate>, evidence: DetectionEvidence) -> DetectionOutcome {
        match matched.len() {
            0 if Self::all_inapplicable(&evidence) => DetectionOutcome::Inapplicable(evidence),
            0 => DetectionOutcome::NoMatch(evidence),
            1 => Self::selected(matched.remove(0), &evidence),
            _ => DetectionOutcome::Ambiguous(AmbiguousDetection {
                candidates: matched,
                evidence,
            }),
        }
    }

    fn unique_closure(evidence: DetectionEvidence) -> DetectionOutcome {
        if let Some(ambiguous) = Self::first_ambiguous(&evidence) {
            return DetectionOutcome::Ambiguous(AmbiguousDetection {
                candidates: ambiguous.candidates,
                evidence,
            });
        }
        let mut matched = Self::matched(&evidence);
        let Some(first) = matched.first() else {
            return if Self::all_inapplicable(&evidence) {
                DetectionOutcome::Inapplicable(evidence)
            } else {
                DetectionOutcome::NoMatch(evidence)
            };
        };
        let points = Self::normalized_points(first);
        if matched
            .iter()
            .all(|candidate| Self::normalized_points(candidate) == points)
        {
            Self::selected(matched.remove(0), &evidence)
        } else {
            DetectionOutcome::Ambiguous(AmbiguousDetection {
                candidates: matched,
                evidence,
            })
        }
    }

    fn select_applicable(evidence: DetectionEvidence) -> DetectionOutcome {
        let applicable = evidence
            .children
            .iter()
            .filter(|outcome| !matches!(outcome, DetectionOutcome::Inapplicable(_)))
            .collect::<Vec<_>>();
        match applicable.as_slice() {
            [] => DetectionOutcome::Inapplicable(evidence),
            [DetectionOutcome::Matched(candidate)] => {
                Self::selected((*candidate).clone(), &evidence)
            }
            [DetectionOutcome::NoMatch(_)] => DetectionOutcome::NoMatch(evidence),
            [DetectionOutcome::Ambiguous(ambiguous)] => {
                DetectionOutcome::Ambiguous(AmbiguousDetection {
                    candidates: ambiguous.candidates.clone(),
                    evidence,
                })
            }
            [DetectionOutcome::Collected(_)] => DetectionOutcome::Collected(evidence),
            [_] => DetectionOutcome::Inapplicable(evidence),
            _ => DetectionOutcome::NoMatch(
                evidence.rejected("multiple detector branches were applicable"),
            ),
        }
    }

    fn selected(
        mut candidate: ProbeCandidate,
        parent_evidence: &DetectionEvidence,
    ) -> DetectionOutcome {
        candidate.evidence.children = parent_evidence.children.clone();
        DetectionOutcome::Matched(candidate)
    }

    fn matched(evidence: &DetectionEvidence) -> Vec<ProbeCandidate> {
        evidence
            .children
            .iter()
            .filter_map(|outcome| match outcome {
                DetectionOutcome::Matched(candidate) => Some(candidate.clone()),
                _ => None,
            })
            .collect()
    }

    fn first_ambiguous(evidence: &DetectionEvidence) -> Option<AmbiguousDetection> {
        evidence.children.iter().find_map(|outcome| match outcome {
            DetectionOutcome::Ambiguous(ambiguous) => Some(ambiguous.clone()),
            _ => None,
        })
    }

    fn ambiguous_executable_by_consumer(ambiguous: &AmbiguousDetection) -> bool {
        ambiguous
            .candidates
            .iter()
            .any(|candidate| candidate.capability.executable_by_consumer())
    }

    fn all_inapplicable(evidence: &DetectionEvidence) -> bool {
        !evidence.children.is_empty()
            && evidence
                .children
                .iter()
                .all(|outcome| matches!(outcome, DetectionOutcome::Inapplicable(_)))
    }

    fn normalized_points(
        candidate: &ProbeCandidate,
    ) -> Vec<(
        std::path::PathBuf,
        crate::BinaryIdentity,
        &'static str,
        String,
        &'static str,
        &'static str,
        u64,
    )> {
        let mut points = candidate
            .points
            .iter()
            .map(|point| {
                (
                    candidate.binary.path.clone(),
                    candidate.binary.identity.clone(),
                    candidate.source.as_str(),
                    point.symbol.clone(),
                    point.direction.as_str(),
                    point.attach.as_str(),
                    point.file_offset,
                )
            })
            .collect::<Vec<_>>();
        points.sort_unstable();
        points
    }
}
