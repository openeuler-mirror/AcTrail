use crate::probe_detector::contract::detection::{DetectionError, DetectionOutcome, ProbeContext};
use crate::probe_detector::contract::identity::DetectorPath;

pub(crate) trait ProbeDetector {
    fn path(&self) -> &DetectorPath;

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError>;
}
