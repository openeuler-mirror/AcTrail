use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::contract::selection::SelectionPolicy;

use super::common_state_pair_27_32::CommonStatePair2732ProbeDetectorConfig;
use super::common_state_pair_27_32_r13::CommonStatePair2732R13ProbeDetectorConfig;
use super::common_state_pair_41_32::CommonStatePair4132ProbeDetectorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct X86_64RustlsProbeDetectorConfig {
    pub(crate) match_limit: usize,
    pub(crate) selection: SelectionPolicy,
    pub(crate) common_state_pair_27_32: CommonStatePair2732ProbeDetectorConfig,
    pub(crate) common_state_pair_27_32_r13: CommonStatePair2732R13ProbeDetectorConfig,
    pub(crate) common_state_pair_41_32: CommonStatePair4132ProbeDetectorConfig,
}

impl X86_64RustlsProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            match_limit,
            selection: SelectionPolicy::UniqueClosure,
            common_state_pair_27_32: CommonStatePair2732ProbeDetectorConfig::default(),
            common_state_pair_27_32_r13: CommonStatePair2732R13ProbeDetectorConfig::default(),
            common_state_pair_41_32: CommonStatePair4132ProbeDetectorConfig::default(),
        }
    }
}

impl ProbeDetectorConfig for X86_64RustlsProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        if self.match_limit == 0 {
            return Err(DetectorConfigError::new(
                "x86_64 Rustls match_limit must be greater than zero",
            ));
        }
        self.common_state_pair_27_32.validate()?;
        self.common_state_pair_27_32_r13.validate()?;
        self.common_state_pair_41_32.validate()
    }
}
