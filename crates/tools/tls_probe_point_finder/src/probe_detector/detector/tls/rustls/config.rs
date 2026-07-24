use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::contract::selection::SelectionPolicy;

use super::static_pattern::RustlsStaticPatternProbeDetectorConfig;
use super::symbol::RustlsSymbolProbeDetectorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RustlsProbeDetectorConfig {
    pub(crate) selection: SelectionPolicy,
    pub(crate) symbol: RustlsSymbolProbeDetectorConfig,
    pub(crate) static_pattern: RustlsStaticPatternProbeDetectorConfig,
}

impl RustlsProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            static_pattern: RustlsStaticPatternProbeDetectorConfig::with_match_limit(match_limit),
            ..Self::default()
        }
    }
}

impl Default for RustlsProbeDetectorConfig {
    fn default() -> Self {
        Self {
            selection: SelectionPolicy::FirstComplete,
            symbol: RustlsSymbolProbeDetectorConfig::default(),
            static_pattern: RustlsStaticPatternProbeDetectorConfig::default(),
        }
    }
}

impl ProbeDetectorConfig for RustlsProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.symbol.validate()?;
        self.static_pattern.validate()
    }
}
