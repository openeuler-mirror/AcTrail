//! BoringSSL entry layouts verified on native ARM64 Bun executables.

use super::probe_detector::{
    EXPANDED_FRAME_READ_INTERNAL_PATTERN, EXPANDED_FRAME_READ_PATTERN, WRITE_PATTERN,
};
use crate::elf::ElfImage;
use crate::probe_detector::detector::tls::boringssl::common::{
    StaticPatternDetection, StaticPatternSupport,
};
use crate::{BinaryIdentityTypeCode, ToolError, ToolResult};

const CLAUDE_WRITE: &[u8] = &[
    0xff, 0x43, 0x01, 0xd1, 0xfd, 0x7b, 0x02, 0xa9, 0xf6, 0x57, 0x03, 0xa9, 0xf4, 0x4f, 0x04, 0xa9,
    0xfd, 0x83, 0x00, 0x91, 0xe8, 0x0c, 0x03, 0xb0, 0x08, 0x7d, 0x47, 0xf9, 0xf5, 0x03, 0x02, 0x2a,
    0xf4, 0x03, 0x01, 0xaa, 0xf3, 0x03, 0x00, 0xaa, 0xa8, 0x83, 0x1f, 0xf8, 0x08, 0x18, 0x40, 0xf9,
    0x1f, 0xc5, 0x00, 0xb9, 0x63, 0x14, 0x0e, 0x94,
];
const CLAUDE_READ_INTERNAL: &[u8] = &[
    0xff, 0xc3, 0x02, 0xd1, 0xfd, 0x7b, 0x05, 0xa9, 0xfb, 0x33, 0x00, 0xf9, 0xfa, 0x67, 0x07, 0xa9,
    0xf8, 0x5f, 0x08, 0xa9, 0xf6, 0x57, 0x09, 0xa9, 0xf4, 0x4f, 0x0a, 0xa9, 0xfd, 0x43, 0x01, 0x91,
    0xe8, 0x0c, 0x03, 0x90, 0x08, 0x7d, 0x47, 0xf9, 0xf3, 0x03, 0x00, 0xaa, 0xa8, 0x83, 0x1f, 0xf8,
    0x08, 0x18, 0x40, 0xf9, 0x1f, 0xc5, 0x00, 0xb9,
];

const PROFILES: &[VerifiedProfile] = &[
    VerifiedProfile {
        // OpenCode 1.19.5. The linker separated SSL_write from the two read entries.
        build_id: "575f3e2cbf93171450778aa621b817e388d140aa",
        write_from_read: 0x7ec00,
        write: WRITE_PATTERN,
        read_internal: EXPANDED_FRAME_READ_INTERNAL_PATTERN,
    },
    VerifiedProfile {
        // Claude Code 2.1.325. Both the write and internal read have stack canaries.
        build_id: "74507c06dbb3aa7767fda21db32e442b13f9ca9a",
        write_from_read: -0x1c0,
        write: CLAUDE_WRITE,
        read_internal: CLAUDE_READ_INTERNAL,
    },
];

pub(super) struct VerifiedProfile {
    build_id: &'static str,
    write_from_read: i64,
    write: &'static [u8],
    read_internal: &'static [u8],
}

impl VerifiedProfile {
    pub(super) fn for_image(image: &ElfImage) -> Option<&'static Self> {
        let identity = image.identity();
        PROFILES.iter().find(|profile| {
            identity.identity_type_code == BinaryIdentityTypeCode::GnuBuildId
                && identity.identity == profile.build_id
        })
    }

    fn patterns(&self) -> [&'static [u8]; 3] {
        [EXPANDED_FRAME_READ_PATTERN, self.read_internal, self.write]
    }

    pub(super) fn register_executable_patterns(&self, image: &ElfImage) {
        for pattern in self.patterns() {
            image.register_pattern_scan(pattern);
        }
    }

    pub(super) fn detect(
        &self,
        image: &ElfImage,
        match_limit: usize,
    ) -> ToolResult<StaticPatternDetection> {
        let patterns = self.patterns();
        let [reads, internals, writes] = image
            .pattern_offsets_for(&patterns, &image.executable_file_ranges()?)
            .try_into()
            .map_err(|_| ToolError::new("invalid BoringSSL pattern results"))?;
        let read = StaticPatternSupport::require_single(&reads, "SSL_read")?;
        let internal = StaticPatternSupport::require_single(&internals, "SSL_read_internal")?;
        let write = StaticPatternSupport::require_single(&writes, "SSL_write")?;
        let offsets = StaticPatternSupport::offsets_with_addresses(
            image,
            &[
                ("SSL_read", read),
                ("SSL_read_internal", internal),
                ("SSL_write", write),
            ],
        )?;
        // Both wrappers call SSL_read_internal at the same instruction offset.
        const READ_CALL_OFFSET: usize = 0x58;
        let call_offset = read + READ_CALL_OFFSET;
        let instruction = image
            .data()
            .get(call_offset..call_offset + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| ToolError::new("missing BoringSSL SSL_read call instruction"))?;
        let call_address = image.virtual_address_for_file_offset(call_offset as u64)?;
        let displacement = i64::from((instruction << 6) as i32 >> 4);
        if instruction & 0xfc000000 != 0x94000000
            || call_address.checked_add_signed(displacement) != Some(offsets[1].virtual_address)
            || offsets[0]
                .virtual_address
                .checked_add_signed(self.write_from_read)
                != Some(offsets[2].virtual_address)
            || !StaticPatternSupport::contains_any(image, &[b"boringssl/ssl/ssl_lib.cc"])
        {
            return Err(ToolError::new(
                "BoringSSL entry relationships did not match the verified build",
            ));
        }
        let matches = offsets
            .iter()
            .zip(patterns)
            .zip([reads, internals, writes])
            .map(|((entry, pattern), found)| {
                StaticPatternSupport::pattern_matches(
                    image,
                    entry.symbol,
                    entry.symbol,
                    pattern,
                    &found,
                    match_limit,
                )
            })
            .collect::<ToolResult<Vec<_>>>()?;
        Ok(StaticPatternDetection {
            arch_label: "aarch64",
            matches,
            offsets,
        })
    }
}
