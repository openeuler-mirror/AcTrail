use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

use super::discovery::OpenSslSharedLibraryDiscoveryProbeDetectorConfig;
use super::symbol::OpenSslSharedLibrarySymbolProbeDetectorConfig;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OpenSslSharedLibraryProbeDetectorConfig {
    pub(crate) discovery: OpenSslSharedLibraryDiscoveryProbeDetectorConfig,
    pub(crate) symbol: OpenSslSharedLibrarySymbolProbeDetectorConfig,
}

impl ProbeDetectorConfig for OpenSslSharedLibraryProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.discovery.validate()?;
        self.symbol.validate()
    }
}
