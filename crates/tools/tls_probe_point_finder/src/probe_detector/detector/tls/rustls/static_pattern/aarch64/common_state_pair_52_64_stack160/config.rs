use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CommonStatePair5264Stack160ProbeDetectorConfig {}

impl ProbeDetectorConfig for CommonStatePair5264Stack160ProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        Ok(())
    }
}
