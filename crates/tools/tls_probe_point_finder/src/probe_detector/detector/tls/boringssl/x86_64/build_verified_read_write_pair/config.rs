use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BuildVerifiedReadWritePairDetectorConfig {
    pub(crate) match_limit: usize,
}

impl ProbeDetectorConfig for BuildVerifiedReadWritePairDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        if self.match_limit == 0 {
            return Err(DetectorConfigError::new(
                "x86_64 BoringSSL verified read/write pair match_limit must be greater than zero",
            ));
        }
        Ok(())
    }
}
