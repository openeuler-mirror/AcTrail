//! Build-specific ARM64 profile for outlined Rustls 0.23.43 functions.

use super::handshake_locator::HandshakeLocation;
use super::static_pattern::{PatternPairProbeDetector, StaticPatternSpec};
use super::{RUNTIME_BUFFER_PLAINTEXT_SYMBOL, RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL};
use crate::elf::ElfImage;
use crate::plan::ProbePoint;
use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::{BinaryIdentity, BinaryIdentityTypeCode};

const BUILD_ID: &str = "bcf409a1125acd7aad8f2f122b272af8df9f4fbb";
const BUFFER_ENTRY: u64 = 0x1fdcc48;
const TAKE_ENTRY: u64 = 0x1fdb404;
const HANDSHAKE_ENTRY: u64 = 0x1fdb328;
const BUFFER: &[u8] = &[
    0xff, 0x83, 0x01, 0xd1, 0xfd, 0x7b, 0x05, 0xa9, 0xfd, 0x43, 0x01, 0x91, 0xe0, 0x0b, 0x00, 0xf9,
    0xe1, 0x07, 0x00, 0xf9, 0xe2, 0x0f, 0x00, 0xf9, 0xe8, 0x03, 0x00, 0xaa, 0xe8, 0x13, 0x00, 0xf9,
    0xe2, 0x17, 0x00, 0xf9, 0x13, 0xfa, 0xff, 0x97, 0xe8, 0x07, 0x40, 0xf9, 0xe0, 0x0b, 0x40, 0xf9,
    0xe3, 0x0f, 0x40, 0xf9, 0x00, 0x01, 0xc0, 0x3d, 0xa1, 0x83, 0x00, 0xd1, 0xa0, 0x03, 0x9e, 0x3c,
    0x00, 0x05, 0xc0, 0x3d, 0xa0, 0x03, 0x9f, 0x3c, 0xe8, 0x03, 0x1f, 0x2a, 0x02, 0x01, 0x00, 0x12,
    0xb4, 0xff, 0xff, 0x97,
];
const TAKE: &[u8] = &[
    0xff, 0x03, 0x02, 0xd1, 0xfd, 0x7b, 0x07, 0xa9, 0xfd, 0xc3, 0x01, 0x91, 0xe0, 0x07, 0x00, 0xf9,
    0xe1, 0x0b, 0x00, 0xf9, 0xa0, 0x83, 0x1e, 0xf8, 0xbf, 0x73, 0x1e, 0x38, 0x28, 0x00, 0x80, 0x52,
    0xa8, 0x73, 0x1e, 0x38, 0x00, 0xb0, 0x0c, 0x91, 0xfc, 0x03, 0x00, 0x94, 0x08, 0x00, 0x00, 0x14,
    0xa8, 0x73, 0x5e, 0x38,
];

// The caller sets may_receive at CommonState + 823 before calling the outgoing
// entry, which sets may_send at +822 and handles the pending plaintext buffer.
const HANDSHAKE_CALLER: &[u32] = &[0x52800028, 0x390cdc08, 0x94000284];
const HANDSHAKE: &[u32] = &[
    0xd10103ff, 0xa9037bfd, 0x9100c3fd, 0xf90007e0, 0xf9000be1, 0xaa0003e8, 0xf9000fe8, 0xaa0103e8,
    0xf81f03a8, 0x52800028, 0x390cd808, 0xf9400028, 0xf1000108,
];
// The certificate chain is stored as three Vec words at CommonState + 0x2f0.
// The Option destructor at 0x1f9e090 ignores the -1 capacity niche.
const CERTIFICATE_OFFSET: u64 = 0x2f0;
const CERTIFICATE_DROP: &[u32] = &[0xf94003e8, 0x910bc100, 0x97ffed3e];
const CERTIFICATE_STORE: &[u32] = &[0x3d80bd20, 0xf94623e8, 0xf9018128];
const CERTIFICATE_OPTION: &[u32] = &[0xf9400008, 0xb1000508, 0x9a9f07e8];

pub(super) struct OutlinedRustlsProfile;

impl OutlinedRustlsProfile {
    pub(super) fn matches(image: &ElfImage) -> bool {
        let identity = image.identity();
        image.arch().as_str() == "aarch64"
            && identity.identity_type_code == BinaryIdentityTypeCode::GnuBuildId
            && identity.identity == BUILD_ID
    }

    pub(super) fn plaintext_pair() -> PatternPairProbeDetector {
        PatternPairProbeDetector::new(
            "aarch64-rustls-common-state-pair-outlined",
            "aarch64",
            [
                StaticPatternSpec {
                    pattern_id: "aarch64-rustls-outlined-buffer-plaintext",
                    symbol: RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
                    bytes: BUFFER,
                },
                StaticPatternSpec {
                    pattern_id: "aarch64-rustls-outlined-take-received-plaintext",
                    symbol: RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
                    bytes: TAKE,
                },
            ],
            vec![VerifiedTarget {
                runtime_version: "Rustls 0.23.43",
                compiler_shape: "aarch64 build with outlined Rustls functions",
                identity: Some(
                    BinaryIdentity::try_new(BinaryIdentityTypeCode::GnuBuildId, BUILD_ID)
                        .expect("verified outlined Rustls build ID"),
                ),
                evidence_source: "entry, caller and certificate layout disassembly",
            }],
        )
    }

    pub(super) fn connection_layout(
        image: &ElfImage,
        points: &[ProbePoint],
    ) -> Option<(HandshakeLocation, u64)> {
        let matches_entry = |symbol: &str, address| {
            points
                .iter()
                .any(|point| point.symbol == symbol && point.virtual_address == address)
        };
        let ranges = image.executable_file_ranges().ok()?;
        let matches_code = |address, expected: &[u32]| {
            let Ok(offset) = image.file_offset_for_virtual_address(address) else {
                return false;
            };
            ranges.iter().any(|(start, bytes)| {
                (offset as usize)
                    .checked_sub(*start)
                    .and_then(|index| bytes.get(index..index.checked_add(expected.len() * 4)?))
                    .is_some_and(|bytes| {
                        bytes
                            .chunks_exact(4)
                            .zip(expected)
                            .all(|(bytes, word)| bytes == word.to_le_bytes())
                    })
            })
        };
        (points.len() == 2
            && matches_entry(RUNTIME_BUFFER_PLAINTEXT_SYMBOL, BUFFER_ENTRY)
            && matches_entry(RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL, TAKE_ENTRY)
            && matches_code(HANDSHAKE_ENTRY, HANDSHAKE)
            && matches_code(0x1fda910, HANDSHAKE_CALLER)
            && matches_code(0x1fa2b90, CERTIFICATE_DROP)
            && matches_code(0x2034088, CERTIFICATE_STORE)
            && matches_code(0x1f9e0a8, CERTIFICATE_OPTION))
        .then_some((
            HandshakeLocation {
                virtual_address: HANDSHAKE_ENTRY,
                send_field: 822,
                preceding_field: 823,
            },
            CERTIFICATE_OFFSET,
        ))
    }
}
