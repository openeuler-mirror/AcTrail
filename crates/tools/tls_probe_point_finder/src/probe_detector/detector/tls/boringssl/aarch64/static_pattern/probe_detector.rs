use crate::elf::ElfImage;
use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::boringssl::common::{
    StaticPatternDetection, StaticPatternSupport,
};
use crate::{ToolError, ToolResult};

use super::Aarch64BoringSslStaticPatternProbeDetectorConfig;

const READ_PATTERN: &[u8] = &[
    0xfd, 0x7b, 0xbd, 0xa9, 0xf5, 0x0b, 0x00, 0xf9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x03, 0x00, 0x91,
    0x08, 0x4c, 0x40, 0xf9, 0xa8, 0x01, 0x00, 0xb4,
];
const READ_INTERNAL_PATTERN: &[u8] = &[
    0xff, 0x03, 0x02, 0xd1, 0xfd, 0x7b, 0x04, 0xa9, 0xf8, 0x5f, 0x05, 0xa9, 0xf6, 0x57, 0x06, 0xa9,
    0xf4, 0x4f, 0x07, 0xa9, 0xfd, 0x03, 0x01, 0x91, 0x08, 0x18, 0x40, 0xf9, 0xf3, 0x03, 0x00, 0xaa,
];
const WRITE_PATTERN: &[u8] = &[
    0xff, 0x03, 0x01, 0xd1, 0xfd, 0x7b, 0x01, 0xa9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03, 0xa9,
    0xfd, 0x43, 0x00, 0x91, 0x08, 0x18, 0x40, 0xf9, 0xf5, 0x03, 0x02, 0x2a, 0xf4, 0x03, 0x01, 0xaa,
];
const WRITE_READ_DELTA: usize = 0x3c0;
const WRITE_READ_INTERNAL_DELTA: usize = 0x2c0;

pub(crate) struct Aarch64BoringSslStaticPatternProbeDetector {
    match_limit: usize,
}

impl Aarch64BoringSslStaticPatternProbeDetector {
    pub(crate) fn try_new(
        config: Aarch64BoringSslStaticPatternProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            match_limit: config.match_limit,
        })
    }

    pub(crate) fn detect(&self, image: &ElfImage) -> ToolResult<StaticPatternDetection> {
        let data = image.data();
        let read_matches = StaticPatternSupport::find_all(data, READ_PATTERN);
        let read_internal_matches = StaticPatternSupport::find_all(data, READ_INTERNAL_PATTERN);
        let write_matches = StaticPatternSupport::find_all(data, WRITE_PATTERN);
        let write = StaticPatternSupport::require_single(&write_matches, "SSL_write")?;
        let read = Self::require_related(
            data,
            &read_matches,
            write,
            READ_PATTERN,
            WRITE_READ_DELTA,
            "SSL_read",
        )?;
        let read_internal = Self::require_related(
            data,
            &read_internal_matches,
            write,
            READ_INTERNAL_PATTERN,
            WRITE_READ_INTERNAL_DELTA,
            "SSL_read_internal",
        )?;
        let offsets = StaticPatternSupport::offsets_with_addresses(
            image,
            &[
                ("SSL_read", read),
                ("SSL_read_internal", read_internal),
                ("SSL_write", write),
            ],
        )?;
        Ok(StaticPatternDetection {
            arch_label: "aarch64",
            matches: vec![
                StaticPatternSupport::pattern_matches(
                    image,
                    "arm64-boringssl-ssl-read-wrapper-24",
                    "SSL_read",
                    READ_PATTERN,
                    &read_matches,
                    self.match_limit,
                )?,
                StaticPatternSupport::pattern_matches(
                    image,
                    "arm64-boringssl-ssl-read-internal-32",
                    "SSL_read_internal",
                    READ_INTERNAL_PATTERN,
                    &read_internal_matches,
                    self.match_limit,
                )?,
                StaticPatternSupport::pattern_matches(
                    image,
                    "arm64-boringssl-ssl-write-entry-32",
                    "SSL_write",
                    WRITE_PATTERN,
                    &write_matches,
                    self.match_limit,
                )?,
            ],
            offsets,
        })
    }

    fn require_related(
        data: &[u8],
        offsets: &[usize],
        write: usize,
        pattern: &[u8],
        delta: usize,
        symbol: &str,
    ) -> ToolResult<usize> {
        let offset = StaticPatternSupport::require_single(offsets, symbol)?;
        let expected = write.checked_sub(delta).ok_or_else(|| {
            ToolError::new(format!(
                "BoringSSL {symbol} offset underflows SSL_write delta"
            ))
        })?;
        if offset == expected && StaticPatternSupport::matches_at(data, expected, pattern) {
            Ok(offset)
        } else {
            Err(ToolError::new(format!(
                "BoringSSL {symbol} is not at SSL_write-0x{delta:x}"
            )))
        }
    }
}
