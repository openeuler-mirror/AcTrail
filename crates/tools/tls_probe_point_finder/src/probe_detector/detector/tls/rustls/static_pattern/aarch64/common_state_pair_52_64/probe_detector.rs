use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::rustls::RustlsConnectionLayout;
use crate::probe_detector::detector::tls::rustls::static_pattern::{
    PatternPairProbeDetector, StaticPatternSpec,
};
use crate::probe_detector::detector::tls::rustls::{
    RUNTIME_BUFFER_PLAINTEXT_SYMBOL, RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
};

use super::CommonStatePair5264ProbeDetectorConfig;
use super::verified_targets::verified_targets;

pub(crate) struct CommonStatePair5264ProbeDetector;

impl CommonStatePair5264ProbeDetector {
    pub(crate) fn try_new(
        config: CommonStatePair5264ProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self)
    }

    pub(crate) fn into_pattern_pair(self) -> PatternPairProbeDetector {
        PatternPairProbeDetector::new(
            "aarch64-rustls-common-state-pair-52-64",
            "aarch64",
            [
                StaticPatternSpec {
                    pattern_id: "aarch64-rustls-common-state-buffer-plaintext-entry-52",
                    symbol: RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
                    bytes: RustlsConnectionLayout::BUFFER_PATTERN,
                },
                StaticPatternSpec {
                    pattern_id: "aarch64-rustls-common-state-take-received-plaintext-entry-64",
                    symbol: RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
                    bytes: RustlsConnectionLayout::TAKE_PATTERN,
                },
            ],
            verified_targets(),
        )
    }
}
