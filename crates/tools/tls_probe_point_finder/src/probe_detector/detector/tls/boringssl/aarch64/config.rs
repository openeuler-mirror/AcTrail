use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

use super::shared_library::Aarch64BoringSslSharedLibraryProbeDetectorConfig;
use super::static_pattern::Aarch64BoringSslStaticPatternProbeDetectorConfig;
use super::symbol::Aarch64BoringSslSymbolProbeDetectorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Aarch64BoringSslProbeDetectorConfig {
    pub(crate) symbol: Aarch64BoringSslSymbolProbeDetectorConfig,
    pub(crate) static_pattern: Aarch64BoringSslStaticPatternProbeDetectorConfig,
    pub(crate) shared_library: Aarch64BoringSslSharedLibraryProbeDetectorConfig,
}

impl Aarch64BoringSslProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            symbol: Aarch64BoringSslSymbolProbeDetectorConfig::default(),
            static_pattern: Aarch64BoringSslStaticPatternProbeDetectorConfig { match_limit },
            shared_library: Aarch64BoringSslSharedLibraryProbeDetectorConfig::default(),
        }
    }
}

impl ProbeDetectorConfig for Aarch64BoringSslProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.symbol.validate()?;
        self.static_pattern.validate()?;
        self.shared_library.validate()
    }
}
