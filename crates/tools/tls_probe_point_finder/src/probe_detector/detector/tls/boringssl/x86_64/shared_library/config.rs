use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct X86_64BoringSslSharedLibraryProbeDetectorConfig {}

impl ProbeDetectorConfig for X86_64BoringSslSharedLibraryProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        Ok(())
    }
}
