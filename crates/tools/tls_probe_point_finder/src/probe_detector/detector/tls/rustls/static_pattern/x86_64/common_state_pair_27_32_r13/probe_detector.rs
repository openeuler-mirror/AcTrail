use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::rustls::static_pattern::{
    PatternPairProbeDetector, StaticPatternSpec,
};
use crate::probe_detector::detector::tls::rustls::{
    RUNTIME_BUFFER_PLAINTEXT_SYMBOL, RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
};

use super::CommonStatePair2732R13ProbeDetectorConfig;
use super::verified_targets::verified_targets;

const BUFFER_PATTERN: &[u8] = &[
    0x55, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec, 0x28, 0x49, 0x89,
    0xd6, 0x48, 0x89, 0xf3, 0x4c, 0x8b, 0xa7, 0x08, 0x03, 0x00, 0x00,
];
const TAKE_PATTERN: &[u8] = &[
    0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x49, 0x89, 0xff, 0xc6, 0x87, 0x2e, 0x03,
    0x00, 0x00, 0x20, 0x4c, 0x8b, 0x2e, 0x4c, 0x8b, 0x76, 0x08, 0x4c, 0x89, 0xe8, 0x48, 0xf7, 0xd8,
];

pub(crate) struct CommonStatePair2732R13ProbeDetector;

impl CommonStatePair2732R13ProbeDetector {
    pub(crate) fn try_new(
        config: CommonStatePair2732R13ProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self)
    }

    pub(crate) fn into_pattern_pair(self) -> PatternPairProbeDetector {
        PatternPairProbeDetector::new(
            "x86_64-rustls-common-state-pair-27-32-r13",
            "x86_64",
            [
                StaticPatternSpec {
                    pattern_id: "x86_64-rustls-common-state-buffer-plaintext-entry-27",
                    symbol: RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
                    bytes: BUFFER_PATTERN,
                },
                StaticPatternSpec {
                    pattern_id: "x86_64-rustls-common-state-take-received-plaintext-entry-32-r13",
                    symbol: RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
                    bytes: TAKE_PATTERN,
                },
            ],
            verified_targets(),
        )
        .requiring_verified_identity()
    }
}
