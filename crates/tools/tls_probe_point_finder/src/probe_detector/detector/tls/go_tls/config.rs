use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

use super::pclntab::GoPclntabProbeDetectorConfig;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GoTlsProbeDetectorConfig {
    pub(crate) pclntab: GoPclntabProbeDetectorConfig,
}

impl ProbeDetectorConfig for GoTlsProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.pclntab.validate()
    }
}
