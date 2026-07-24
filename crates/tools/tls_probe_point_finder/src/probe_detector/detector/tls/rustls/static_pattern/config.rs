use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::contract::selection::SelectionPolicy;

use super::aarch64::Aarch64RustlsProbeDetectorConfig;
use super::x86_64::X86_64RustlsProbeDetectorConfig;

pub(crate) const DEFAULT_MATCH_LIMIT: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RustlsStaticPatternProbeDetectorConfig {
    pub(crate) selection: SelectionPolicy,
    pub(crate) x86_64: X86_64RustlsProbeDetectorConfig,
    pub(crate) aarch64: Aarch64RustlsProbeDetectorConfig,
}

impl RustlsStaticPatternProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            selection: SelectionPolicy::SelectApplicable,
            x86_64: X86_64RustlsProbeDetectorConfig::with_match_limit(match_limit),
            aarch64: Aarch64RustlsProbeDetectorConfig::with_match_limit(match_limit),
        }
    }
}

impl Default for RustlsStaticPatternProbeDetectorConfig {
    fn default() -> Self {
        Self::with_match_limit(DEFAULT_MATCH_LIMIT)
    }
}

impl ProbeDetectorConfig for RustlsStaticPatternProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.x86_64.validate()?;
        self.aarch64.validate()
    }
}
