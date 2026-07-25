use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::rustls::static_pattern::{
    PatternPairProbeDetector, StaticPatternSpec,
};
use crate::probe_detector::detector::tls::rustls::{
    RUNTIME_BUFFER_PLAINTEXT_SYMBOL, RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
};

use super::CommonStatePair4856ProbeDetectorConfig;
use super::verified_targets::verified_targets;

const BUFFER_PATTERN: &[u8] = &[
    0xfd, 0x7b, 0xbc, 0xa9, 0xf8, 0x5f, 0x01, 0xa9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03, 0xa9,
    0xfd, 0x03, 0x00, 0x91, 0x17, 0x84, 0x41, 0xf9, 0x08, 0x00, 0xf0, 0xd2, 0xf5, 0x03, 0x02, 0xaa,
    0xf3, 0x03, 0x01, 0xaa, 0xf4, 0x03, 0x00, 0xaa, 0x08, 0x84, 0x01, 0xf9, 0xff, 0x02, 0x08, 0xeb,
];
const TAKE_PATTERN: &[u8] = &[
    0xfd, 0x7b, 0xbd, 0xa9, 0xf6, 0x57, 0x01, 0xa9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x03, 0x00, 0x91,
    0x36, 0x50, 0x40, 0xa9, 0x09, 0x00, 0xf0, 0xd2, 0x33, 0x08, 0x40, 0xf9, 0xf5, 0x03, 0x00, 0xaa,
    0x08, 0x04, 0x80, 0x52, 0x08, 0xb8, 0x0c, 0x39, 0xdf, 0x02, 0x09, 0xeb, 0x81, 0x02, 0x00, 0x54,
    0xd3, 0x00, 0xf8, 0xb6, 0xe0, 0x03, 0x1f, 0xaa,
];

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
                    bytes: BUFFER_PATTERN,
                },
                StaticPatternSpec {
                    pattern_id: "aarch64-rustls-common-state-take-received-plaintext-entry-56",
                    symbol: RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
                    bytes: TAKE_PATTERN,
                },
            ],
            verified_targets(),
        )
    }
}
