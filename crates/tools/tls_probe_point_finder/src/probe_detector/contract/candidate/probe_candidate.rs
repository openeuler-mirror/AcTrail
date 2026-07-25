use crate::plan::{
    ProbeBinary, ProbePoint, ProbePointPlan, ProbeSource, TargetIdentity, TlsProvider,
};
use crate::probe_detector::contract::capability::DetectorCapability;
use crate::probe_detector::contract::detection::DetectionEvidence;
use crate::probe_detector::contract::identity::DetectorPath;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProbeCandidate {
    pub(crate) detector_path: DetectorPath,
    pub(crate) target: TargetIdentity,
    pub(crate) provider: TlsProvider,
    pub(crate) source: ProbeSource,
    pub(crate) binary: ProbeBinary,
    pub(crate) resolver: String,
    pub(crate) points: Vec<ProbePoint>,
    pub(crate) evidence: DetectionEvidence,
    pub(crate) capability: DetectorCapability,
}

impl ProbeCandidate {
    pub(crate) fn into_plan(self) -> ProbePointPlan {
        ProbePointPlan {
            target: self.target,
            provider: self.provider,
            source: self.source,
            resolver: self.resolver,
            binary: self.binary,
            points: self.points,
        }
    }
}
