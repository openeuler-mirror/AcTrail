use crate::elf::ElfImage;
use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::boringssl::common::{
    StaticPatternDetection, StaticPatternSupport,
};
use crate::{ToolError, ToolResult};

use super::Aarch64BoringSslStaticPatternProbeDetectorConfig;

const LEGACY_READ_PATTERN: &[u8] = &[
    0xfd, 0x7b, 0xbd, 0xa9, 0xf5, 0x0b, 0x00, 0xf9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x03, 0x00, 0x91,
    0x08, 0x4c, 0x40, 0xf9, 0xa8, 0x01, 0x00, 0xb4,
];
const LEGACY_READ_INTERNAL_PATTERN: &[u8] = &[
    0xff, 0x03, 0x02, 0xd1, 0xfd, 0x7b, 0x04, 0xa9, 0xf8, 0x5f, 0x05, 0xa9, 0xf6, 0x57, 0x06, 0xa9,
    0xf4, 0x4f, 0x07, 0xa9, 0xfd, 0x03, 0x01, 0x91, 0x08, 0x18, 0x40, 0xf9, 0xf3, 0x03, 0x00, 0xaa,
];
const EXPANDED_FRAME_READ_PATTERN: &[u8] = &[
    0xfd, 0x7b, 0xbd, 0xa9, 0xf5, 0x0b, 0x00, 0xf9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x03, 0x00, 0x91,
    0x08, 0x4c, 0x40, 0xf9, 0xc8, 0x01, 0x00, 0xb4,
];
const EXPANDED_FRAME_READ_INTERNAL_PATTERN: &[u8] = &[
    0xff, 0x43, 0x02, 0xd1, 0xfd, 0x7b, 0x03, 0xa9, 0xfb, 0x23, 0x00, 0xf9, 0xfa, 0x67, 0x05, 0xa9,
    0xf8, 0x5f, 0x06, 0xa9, 0xf6, 0x57, 0x07, 0xa9, 0xf4, 0x4f, 0x08, 0xa9, 0xfd, 0xc3, 0x00, 0x91,
    0x08, 0x18, 0x40, 0xf9, 0xf3, 0x03, 0x00, 0xaa,
];
const WRITE_PATTERN: &[u8] = &[
    0xff, 0x03, 0x01, 0xd1, 0xfd, 0x7b, 0x01, 0xa9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03, 0xa9,
    0xfd, 0x43, 0x00, 0x91, 0x08, 0x18, 0x40, 0xf9, 0xf5, 0x03, 0x02, 0x2a, 0xf4, 0x03, 0x01, 0xaa,
];

const STATIC_PATTERN_PROFILES: &[StaticPatternProfile] = &[
    StaticPatternProfile {
        read_pattern_id: "arm64-boringssl-ssl-read-wrapper-24",
        read_pattern: LEGACY_READ_PATTERN,
        read_internal_pattern_id: "arm64-boringssl-ssl-read-internal-32",
        read_internal_pattern: LEGACY_READ_INTERNAL_PATTERN,
        write_read_delta: 0x3c0,
        write_read_internal_delta: 0x2c0,
    },
    StaticPatternProfile {
        read_pattern_id: "arm64-boringssl-ssl-read-wrapper-expanded-frame-24",
        read_pattern: EXPANDED_FRAME_READ_PATTERN,
        read_internal_pattern_id: "arm64-boringssl-ssl-read-internal-expanded-frame-40",
        read_internal_pattern: EXPANDED_FRAME_READ_INTERNAL_PATTERN,
        write_read_delta: 0x400,
        write_read_internal_delta: 0x300,
    },
];

struct StaticPatternProfile {
    read_pattern_id: &'static str,
    read_pattern: &'static [u8],
    read_internal_pattern_id: &'static str,
    read_internal_pattern: &'static [u8],
    write_read_delta: usize,
    write_read_internal_delta: usize,
}

struct ResolvedProfile {
    profile: &'static StaticPatternProfile,
    read: usize,
    read_internal: usize,
}

impl StaticPatternProfile {
    fn resolve(
        &'static self,
        data: &[u8],
        executable_ranges: &[(usize, &[u8])],
        write: usize,
    ) -> Option<ResolvedProfile> {
        let read = write.checked_sub(self.write_read_delta)?;
        let read_internal = write.checked_sub(self.write_read_internal_delta)?;
        if !Self::matches_executable_at(data, executable_ranges, read, self.read_pattern)
            || !Self::matches_executable_at(
                data,
                executable_ranges,
                read_internal,
                self.read_internal_pattern,
            )
        {
            return None;
        }
        Some(ResolvedProfile {
            profile: self,
            read,
            read_internal,
        })
    }

    fn matches_executable_at(
        data: &[u8],
        executable_ranges: &[(usize, &[u8])],
        offset: usize,
        pattern: &[u8],
    ) -> bool {
        let Some(end) = offset.checked_add(pattern.len()) else {
            return false;
        };
        executable_ranges.iter().any(|(range_offset, range)| {
            range_offset
                .checked_add(range.len())
                .is_some_and(|range_end| offset >= *range_offset && end <= range_end)
        }) && StaticPatternSupport::matches_at(data, offset, pattern)
    }
}

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

    pub(crate) fn register_executable_patterns(&self, image: &ElfImage) {
        image.register_pattern_scan(WRITE_PATTERN);
    }

    pub(crate) fn detect(&self, image: &ElfImage) -> ToolResult<StaticPatternDetection> {
        let data = image.data();
        let executable_ranges = image.executable_file_ranges()?;
        let write_matches = image
            .pattern_offsets_for(&[WRITE_PATTERN], &executable_ranges)
            .remove(0);
        let write = StaticPatternSupport::require_single(&write_matches, "SSL_write")?;
        let resolved = Self::resolve_profile(data, &executable_ranges, write)?;
        let offsets = StaticPatternSupport::offsets_with_addresses(
            image,
            &[
                ("SSL_read", resolved.read),
                ("SSL_read_internal", resolved.read_internal),
                ("SSL_write", write),
            ],
        )?;
        Ok(StaticPatternDetection {
            arch_label: "aarch64",
            matches: vec![
                StaticPatternSupport::pattern_matches(
                    image,
                    resolved.profile.read_pattern_id,
                    "SSL_read",
                    resolved.profile.read_pattern,
                    std::slice::from_ref(&resolved.read),
                    self.match_limit,
                )?,
                StaticPatternSupport::pattern_matches(
                    image,
                    resolved.profile.read_internal_pattern_id,
                    "SSL_read_internal",
                    resolved.profile.read_internal_pattern,
                    std::slice::from_ref(&resolved.read_internal),
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

    fn resolve_profile(
        data: &[u8],
        executable_ranges: &[(usize, &[u8])],
        write: usize,
    ) -> ToolResult<ResolvedProfile> {
        let mut resolved = None;
        for profile in STATIC_PATTERN_PROFILES {
            let Some(candidate) = profile.resolve(data, executable_ranges, write) else {
                continue;
            };
            if resolved.is_some() {
                return Err(ToolError::new(
                    "multiple aarch64 BoringSSL static pattern profiles matched",
                ));
            }
            resolved = Some(candidate);
        }
        resolved.ok_or_else(|| {
            ToolError::new("no aarch64 BoringSSL static pattern profile matched SSL_write")
        })
    }
}
