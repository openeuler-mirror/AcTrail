use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

use super::shared_library::NssSharedLibraryProbeDetectorConfig;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct NssProbeDetectorConfig {
    pub(crate) shared_library: NssSharedLibraryProbeDetectorConfig,
}

impl ProbeDetectorConfig for NssProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.shared_library.validate()
    }
}
