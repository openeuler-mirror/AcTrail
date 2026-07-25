use crate::elf::ElfImage;
use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::boringssl::common::{
    StaticPatternDetection, StaticPatternSupport,
};
use crate::{ToolError, ToolResult};

use super::X86_64BoringSslStaticPatternProbeDetectorConfig;

const HANDSHAKE_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec,
    0x28, 0x49, 0x89, 0xfc, 0x48, 0x8b, 0x47, 0x30,
];
const READ_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x53, 0x50, 0x48, 0x83, 0xbf, 0x98, 0x00, 0x00,
    0x00, 0x00, 0x74,
];
const WRITE_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec,
    0x18, 0x41, 0x89, 0xd7, 0x49, 0x89, 0xf6, 0x48, 0x89, 0xfb,
];
const IDENTITY_MARKERS: &[&[u8]] = &[
    b"vendor/boringssl/",
    b"OPENSSL_internal",
    b"BoringSSLError",
    b"openssl_is_boringssl",
];
const READ_HANDSHAKE_DELTA: usize = 0x6f0;
const WRITE_READ_DELTA: usize = 0xca0;
const WRITE_SEARCH_WINDOW: usize = 0x10000;

pub(crate) struct X86_64BoringSslStaticPatternProbeDetector {
    match_limit: usize,
}

struct ResolvedOffsets {
    handshake: Option<usize>,
    read: usize,
    write: usize,
}

impl X86_64BoringSslStaticPatternProbeDetector {
    pub(crate) fn try_new(
        config: X86_64BoringSslStaticPatternProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            match_limit: config.match_limit,
        })
    }

    pub(crate) fn detect(&self, image: &ElfImage) -> ToolResult<StaticPatternDetection> {
        let data = image.data();
        let handshake_matches = StaticPatternSupport::find_all(data, HANDSHAKE_PATTERN);
        let read_matches = StaticPatternSupport::find_all(data, READ_PATTERN);
        let write_matches = StaticPatternSupport::find_all(data, WRITE_PATTERN);
        let resolved =
            Self::resolve_offsets(data, &handshake_matches, &read_matches, &write_matches)?;
        let mut resolved_offsets = Vec::new();
        if let Some(handshake) = resolved.handshake {
            resolved_offsets.push(("SSL_do_handshake", handshake));
        }
        resolved_offsets.push(("SSL_read", resolved.read));
        resolved_offsets.push(("SSL_write", resolved.write));
        let offsets = StaticPatternSupport::offsets_with_addresses(image, &resolved_offsets)?;
        Ok(StaticPatternDetection {
            arch_label: "x86_64",
            matches: vec![
                StaticPatternSupport::pattern_matches(
                    image,
                    "x86_64-boringssl-ssl-do-handshake-entry-24",
                    "SSL_do_handshake",
                    HANDSHAKE_PATTERN,
                    &handshake_matches,
                    self.match_limit,
                )?,
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
                    "x86_64-boringssl-ssl-write-entry-26",
                    "SSL_write",
                    WRITE_PATTERN,
                    &write_matches,
                    self.match_limit,
                )?,
            ],
            offsets,
        })
    }

    fn resolve_offsets(
        data: &[u8],
        handshake_matches: &[usize],
        read_matches: &[usize],
        write_matches: &[usize],
    ) -> ToolResult<ResolvedOffsets> {
        let read = StaticPatternSupport::require_single(read_matches, "SSL_read")?;
        let write = Self::resolve_write(data, write_matches, read)?;
        let handshake = match Self::resolve_handshake(data, handshake_matches, read) {
            Ok(offset) => Some(offset),
            Err(_) if Self::has_identity(data) => None,
            Err(error) => return Err(error),
        };
        Ok(ResolvedOffsets {
            handshake,
            read,
            write,
        })
    }

    fn resolve_handshake(data: &[u8], matches: &[usize], read: usize) -> ToolResult<usize> {
        if let Some(expected) = read.checked_sub(READ_HANDSHAKE_DELTA)
            && StaticPatternSupport::matches_at(data, expected, HANDSHAKE_PATTERN)
        {
            return Ok(expected);
        }
        StaticPatternSupport::require_single(matches, "SSL_do_handshake")
    }

    fn resolve_write(data: &[u8], matches: &[usize], read: usize) -> ToolResult<usize> {
        let expected = read
            .checked_add(WRITE_READ_DELTA)
            .ok_or_else(|| ToolError::new("BoringSSL SSL_write expected offset overflow"))?;
        if StaticPatternSupport::matches_at(data, expected, WRITE_PATTERN) {
            return Ok(expected);
        }
        let search_end = data.len().min(read.saturating_add(WRITE_SEARCH_WINDOW));
        let nearby = StaticPatternSupport::find_all(&data[read..search_end], WRITE_PATTERN)
            .into_iter()
            .map(|offset| read + offset)
            .collect::<Vec<_>>();
        if nearby.len() == 1 {
            return Ok(nearby[0]);
        }
        if matches.len() == 1 {
            return Ok(matches[0]);
        }
        Err(ToolError::new(format!(
            "BoringSSL SSL_write nearby pattern match count={}",
            nearby.len()
        )))
    }

    fn has_identity(data: &[u8]) -> bool {
        let vendor = StaticPatternSupport::contains(data, IDENTITY_MARKERS[0]);
        let openssl_error = StaticPatternSupport::contains(data, IDENTITY_MARKERS[1]);
        let boring_error = StaticPatternSupport::contains(data, IDENTITY_MARKERS[2]);
        let flag = StaticPatternSupport::contains(data, IDENTITY_MARKERS[3]);
        vendor || flag || (openssl_error && boring_error)
    }
}
