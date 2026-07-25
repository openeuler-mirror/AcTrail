use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Aarch64BoringSslSymbolProbeDetectorConfig {}

impl ProbeDetectorConfig for Aarch64BoringSslSymbolProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        Ok(())
    }
}
