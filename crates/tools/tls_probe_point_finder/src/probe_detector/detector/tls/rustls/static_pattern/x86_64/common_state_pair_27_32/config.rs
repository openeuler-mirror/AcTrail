use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CommonStatePair2732ProbeDetectorConfig {}

impl ProbeDetectorConfig for CommonStatePair2732ProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        Ok(())
    }
}
