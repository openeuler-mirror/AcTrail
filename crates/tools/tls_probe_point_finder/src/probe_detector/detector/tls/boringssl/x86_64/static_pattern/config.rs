use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct X86_64BoringSslStaticPatternProbeDetectorConfig {
    pub(crate) match_limit: usize,
}

impl ProbeDetectorConfig for X86_64BoringSslStaticPatternProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        if self.match_limit == 0 {
            return Err(DetectorConfigError::new(
                "x86_64 BoringSSL match_limit must be greater than zero",
            ));
        }
        Ok(())
    }
}
