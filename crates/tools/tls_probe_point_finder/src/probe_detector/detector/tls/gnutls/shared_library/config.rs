use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GnuTlsSharedLibraryProbeDetectorConfig {}

impl ProbeDetectorConfig for GnuTlsSharedLibraryProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        Ok(())
    }
}
