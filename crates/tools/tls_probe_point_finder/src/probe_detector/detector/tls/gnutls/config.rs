use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

use super::shared_library::GnuTlsSharedLibraryProbeDetectorConfig;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GnuTlsProbeDetectorConfig {
    pub(crate) shared_library: GnuTlsSharedLibraryProbeDetectorConfig,
}

impl ProbeDetectorConfig for GnuTlsProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.shared_library.validate()
    }
}
