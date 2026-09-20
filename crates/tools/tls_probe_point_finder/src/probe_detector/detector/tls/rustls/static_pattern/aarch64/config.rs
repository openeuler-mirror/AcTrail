use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::contract::selection::SelectionPolicy;

use super::common_state_pair_48_56::CommonStatePair4856ProbeDetectorConfig;
use super::common_state_pair_52_64::CommonStatePair5264ProbeDetectorConfig;
use super::common_state_pair_52_64_x22::CommonStatePair5264X22ProbeDetectorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Aarch64RustlsProbeDetectorConfig {
    pub(crate) match_limit: usize,
    pub(crate) selection: SelectionPolicy,
    pub(crate) common_state_pair_52_64: CommonStatePair5264ProbeDetectorConfig,
    pub(crate) common_state_pair_48_56: CommonStatePair4856ProbeDetectorConfig,
    pub(crate) common_state_pair_52_64_x22: CommonStatePair5264X22ProbeDetectorConfig,
}

impl Aarch64RustlsProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            match_limit,
            selection: SelectionPolicy::UniqueClosure,
            common_state_pair_52_64: CommonStatePair5264ProbeDetectorConfig::default(),
            common_state_pair_48_56: CommonStatePair4856ProbeDetectorConfig::default(),
            common_state_pair_52_64_x22: CommonStatePair5264X22ProbeDetectorConfig::default(),
        }
    }
}

impl ProbeDetectorConfig for Aarch64RustlsProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        if self.match_limit == 0 {
            return Err(DetectorConfigError::new(
                "aarch64 Rustls match_limit must be greater than zero",
            ));
        }
        self.common_state_pair_52_64.validate()?;
        self.common_state_pair_48_56.validate()?;
        self.common_state_pair_52_64_x22.validate()
    }
}
