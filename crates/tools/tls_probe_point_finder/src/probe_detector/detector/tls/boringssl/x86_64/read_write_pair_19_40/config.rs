use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadWritePair1940ProbeDetectorConfig {
    pub(crate) match_limit: usize,
}

impl ProbeDetectorConfig for ReadWritePair1940ProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        if self.match_limit == 0 {
            return Err(DetectorConfigError::new(
                "x86_64 BoringSSL read19/write40 match_limit must be greater than zero",
            ));
        }
        Ok(())
    }
}
