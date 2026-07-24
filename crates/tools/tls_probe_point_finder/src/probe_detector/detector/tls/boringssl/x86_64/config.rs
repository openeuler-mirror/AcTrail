use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

use super::shared_library::X86_64BoringSslSharedLibraryProbeDetectorConfig;
use super::static_pattern::X86_64BoringSslStaticPatternProbeDetectorConfig;
use super::symbol::X86_64BoringSslSymbolProbeDetectorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct X86_64BoringSslProbeDetectorConfig {
    pub(crate) symbol: X86_64BoringSslSymbolProbeDetectorConfig,
    pub(crate) static_pattern: X86_64BoringSslStaticPatternProbeDetectorConfig,
    pub(crate) shared_library: X86_64BoringSslSharedLibraryProbeDetectorConfig,
}

impl X86_64BoringSslProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            symbol: X86_64BoringSslSymbolProbeDetectorConfig::default(),
            static_pattern: X86_64BoringSslStaticPatternProbeDetectorConfig { match_limit },
            shared_library: X86_64BoringSslSharedLibraryProbeDetectorConfig::default(),
        }
    }
}

impl ProbeDetectorConfig for X86_64BoringSslProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.symbol.validate()?;
        self.static_pattern.validate()?;
        self.shared_library.validate()
    }
}
