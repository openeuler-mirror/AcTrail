use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::rustls::static_pattern::{
    PatternPairProbeDetector, StaticPatternSpec,
};
use crate::probe_detector::detector::tls::rustls::{
    RUNTIME_BUFFER_PLAINTEXT_SYMBOL, RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL, RustlsConnectionLayout,
};

use super::CommonStatePair4856ProbeDetectorConfig;
use super::verified_targets::verified_targets;

pub(crate) struct CommonStatePair4856ProbeDetector;

impl CommonStatePair4856ProbeDetector {
    pub(crate) fn try_new(
        config: CommonStatePair4856ProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self)
    }

    pub(crate) fn into_pattern_pair(self) -> PatternPairProbeDetector {
        PatternPairProbeDetector::new(
            "aarch64-rustls-common-state-pair-48-56",
            "aarch64",
            [
                StaticPatternSpec {
                    pattern_id: "aarch64-rustls-common-state-buffer-plaintext-entry-48",
                    symbol: RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
                    bytes: RustlsConnectionLayout::BUFFER_48_PATTERN,
                },
                StaticPatternSpec {
                    pattern_id: "aarch64-rustls-common-state-take-received-plaintext-entry-56",
                    symbol: RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
                    bytes: RustlsConnectionLayout::TAKE_56_PATTERN,
                },
            ],
            verified_targets(),
        )
    }
}
