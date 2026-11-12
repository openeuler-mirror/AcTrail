use crate::elf::ElfImage;
use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::boringssl::common::{
    StaticPatternDetection, StaticPatternSupport,
};
use crate::probe_detector::detector::tls::boringssl::x86_64::static_pattern::X86_64BoringSslStaticPatternProbeDetector;
use crate::{BinaryIdentityTypeCode, ToolError, ToolResult};

use super::ReadWritePair1940ProbeDetectorConfig;

const VERIFIED_BUILD_ID: &str = "588d7307f333293c728b74df19b7323c20736899";

const READ_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x53, 0x50, 0x48, 0x83, 0xbf, 0x98, 0x00, 0x00,
    0x00, 0x00, 0x74,
];
const WRITE_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec,
    0x18, 0x41, 0x89, 0xd7, 0x49, 0x89, 0xf6, 0x48, 0x89, 0xfb, 0x48, 0x8b, 0x47, 0x30, 0xc7, 0x80,
    0xc4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

pub(crate) struct ReadWritePair1940ProbeDetector {
    match_limit: usize,
}

impl ReadWritePair1940ProbeDetector {
    pub(crate) fn try_new(
        config: ReadWritePair1940ProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            match_limit: config.match_limit,
        })
    }

    pub(crate) fn register_executable_patterns(&self, image: &ElfImage) {
        if !Self::matches_build(image) {
            return;
        }
        image.register_pattern_scan(READ_PATTERN);
        image.register_pattern_scan(WRITE_PATTERN);
    }

    pub(crate) fn detect(&self, image: &ElfImage) -> ToolResult<Option<StaticPatternDetection>> {
        if !Self::matches_build(image) {
            return Ok(None);
        }
        let read_matches = StaticPatternSupport::find_all_executable(image, READ_PATTERN)?;
        let read = StaticPatternSupport::require_single(&read_matches, "SSL_read")?;
        let write_matches = StaticPatternSupport::find_all_executable(image, WRITE_PATTERN)?;
        let write = StaticPatternSupport::require_single(&write_matches, "SSL_write")?;
        if !X86_64BoringSslStaticPatternProbeDetector::has_identity(image) {
            return Err(ToolError::new("BoringSSL identity markers were not found"));
        }
        Ok(Some(StaticPatternDetection {
            arch_label: "x86_64",
            matches: vec![
                StaticPatternSupport::pattern_matches(
                    image,
                    "x86_64-boringssl-ssl-read-entry-19",
                    "SSL_read",
                    READ_PATTERN,
                    &read_matches,
                    self.match_limit,
                )?,
                StaticPatternSupport::pattern_matches(
                    image,
                    "x86_64-boringssl-ssl-write-entry-40",
                    "SSL_write",
                    WRITE_PATTERN,
                    &write_matches,
                    self.match_limit,
                )?,
            ],
            offsets: StaticPatternSupport::offsets_with_addresses(
                image,
                &[("SSL_read", read), ("SSL_write", write)],
            )?,
        }))
    }

    fn matches_build(image: &ElfImage) -> bool {
        let identity = image.identity();
        identity.identity_type_code == BinaryIdentityTypeCode::GnuBuildId
            && identity.identity == VERIFIED_BUILD_ID
    }
}
