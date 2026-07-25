use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OpenSslSharedLibrarySymbolProbeDetectorConfig {}

impl ProbeDetectorConfig for OpenSslSharedLibrarySymbolProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        Ok(())
    }
}
