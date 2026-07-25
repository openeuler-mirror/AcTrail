use crate::probe_detector::contract::candidate::ProbeCandidate;
use crate::probe_detector::contract::detection::DetectionEvidence;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AmbiguousDetection {
    pub(crate) candidates: Vec<ProbeCandidate>,
    pub(crate) evidence: DetectionEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DetectionOutcome {
    Inapplicable(DetectionEvidence),
    NoMatch(DetectionEvidence),
    Matched(ProbeCandidate),
    Ambiguous(AmbiguousDetection),
    Collected(DetectionEvidence),
}
