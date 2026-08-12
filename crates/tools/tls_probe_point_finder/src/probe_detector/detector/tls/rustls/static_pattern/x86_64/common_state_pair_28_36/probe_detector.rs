use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::rustls::static_pattern::{
    PatternPairProbeDetector, StaticPatternSpec,
};
use crate::probe_detector::detector::tls::rustls::{
    RUNTIME_BUFFER_PLAINTEXT_SYMBOL, RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
};

use super::CommonStatePair2836ProbeDetectorConfig;
use super::verified_targets::verified_targets;

const BUFFER_PATTERN: &[u8] = &[
    0x48, 0x83, 0xec, 0x28, 0x48, 0x89, 0x3c, 0x24, 0x48, 0x89, 0x74, 0x24, 0x08, 0x48, 0x89, 0x54,
    0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x20,
];
const TAKE_PATTERN: &[u8] = &[
    0x48, 0x83, 0xec, 0x68, 0x48, 0x89, 0x7c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89,
    0x7c, 0x24, 0x50, 0xc6, 0x44, 0x24, 0x4f, 0x00, 0xc6, 0x44, 0x24, 0x4f, 0x01, 0x48, 0x81, 0xc7,
    0x2c, 0x03, 0x00, 0x00,
];

pub(crate) struct CommonStatePair2836ProbeDetector;

impl CommonStatePair2836ProbeDetector {
    pub(crate) fn try_new(
        config: CommonStatePair2836ProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self)
    }

    pub(crate) fn into_pattern_pair(self) -> PatternPairProbeDetector {
        PatternPairProbeDetector::new(
            "x86_64-rustls-common-state-pair-28-36",
            "x86_64",
            [
                StaticPatternSpec {
                    pattern_id: "x86_64-rustls-common-state-buffer-plaintext-entry-28",
                    symbol: RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
                    bytes: BUFFER_PATTERN,
                },
                StaticPatternSpec {
                    pattern_id: "x86_64-rustls-common-state-take-received-plaintext-entry-36",
                    symbol: RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
                    bytes: TAKE_PATTERN,
                },
            ],
            verified_targets(),
        )
    }
}
