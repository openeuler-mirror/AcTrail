use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::contract::selection::SelectionPolicy;

use super::read_write_pair_19_40::ReadWritePair1940ProbeDetectorConfig;
use super::shared_library::X86_64BoringSslSharedLibraryProbeDetectorConfig;
use super::static_pattern::X86_64BoringSslStaticPatternProbeDetectorConfig;
use super::symbol::X86_64BoringSslSymbolProbeDetectorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct X86_64BoringSslProbeDetectorConfig {
    pub(crate) symbol: X86_64BoringSslSymbolProbeDetectorConfig,
    pub(crate) static_pattern: X86_64BoringSslStaticPatternProbeDetectorConfig,
    pub(crate) read_write_pair_19_40: ReadWritePair1940ProbeDetectorConfig,
    pub(crate) static_selection: SelectionPolicy,
    pub(crate) shared_library: X86_64BoringSslSharedLibraryProbeDetectorConfig,
}

impl X86_64BoringSslProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            symbol: X86_64BoringSslSymbolProbeDetectorConfig::default(),
            static_pattern: X86_64BoringSslStaticPatternProbeDetectorConfig { match_limit },
            read_write_pair_19_40: ReadWritePair1940ProbeDetectorConfig { match_limit },
            static_selection: SelectionPolicy::UniqueClosure,
            shared_library: X86_64BoringSslSharedLibraryProbeDetectorConfig::default(),
        }
    }
}

impl ProbeDetectorConfig for X86_64BoringSslProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.symbol.validate()?;
        self.static_pattern.validate()?;
        self.read_write_pair_19_40.validate()?;
        self.shared_library.validate()
    }
}
