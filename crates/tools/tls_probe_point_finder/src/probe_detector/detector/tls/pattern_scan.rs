//! Shared executable pattern-scan registration for TLS detectors.
//!
//! Pattern-based detectors register every pattern they may scan so the first
//! actual scan request covers all of them in one pass. Detectors that cannot
//! apply to the current context skip registration entirely.

use crate::probe_detector::contract::detection::ProbeContext;
use crate::probe_detector::contract::identity::DetectorPath;

pub(crate) trait ExecutablePatternRegistration {
    fn register_executable_patterns(&self, context: &ProbeContext<'_>);
    fn detector_path(&self) -> &DetectorPath;
}
