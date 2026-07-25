use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RustlsSymbolProbeDetectorConfig {}

impl ProbeDetectorConfig for RustlsSymbolProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        Ok(())
    }
}
