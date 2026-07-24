use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

use super::aarch64::Aarch64BoringSslProbeDetectorConfig;
use super::x86_64::X86_64BoringSslProbeDetectorConfig;

pub(crate) const DEFAULT_MATCH_LIMIT: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoringSslProbeDetectorConfig {
    pub(crate) x86_64: X86_64BoringSslProbeDetectorConfig,
    pub(crate) aarch64: Aarch64BoringSslProbeDetectorConfig,
}

impl BoringSslProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            x86_64: X86_64BoringSslProbeDetectorConfig::with_match_limit(match_limit),
            aarch64: Aarch64BoringSslProbeDetectorConfig::with_match_limit(match_limit),
        }
    }
}

impl Default for BoringSslProbeDetectorConfig {
    fn default() -> Self {
        Self::with_match_limit(DEFAULT_MATCH_LIMIT)
    }
}

impl ProbeDetectorConfig for BoringSslProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.x86_64.validate()?;
        self.aarch64.validate()
    }
}
