use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

use super::common::executable::OpenSslExecutableProbeDetectorConfig;
use super::common::shared_library::OpenSslSharedLibraryProbeDetectorConfig;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OpenSslProbeDetectorConfig {
    pub(crate) executable: OpenSslExecutableProbeDetectorConfig,
    pub(crate) shared_library: OpenSslSharedLibraryProbeDetectorConfig,
}

impl ProbeDetectorConfig for OpenSslProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.executable.validate()?;
        self.shared_library.validate()
    }
}
