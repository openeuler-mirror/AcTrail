use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Aarch64BoringSslSharedLibraryProbeDetectorConfig {}

impl ProbeDetectorConfig for Aarch64BoringSslSharedLibraryProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        Ok(())
    }
}
