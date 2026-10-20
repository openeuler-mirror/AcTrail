//! Certificate ownership layout is checked independently of handshake discovery.
//! The destructor shapes were verified on rustls 0.23.36, 0.23.45 and 0.23.46 client builds.
use super::handshake_locator::RustlsHandshakeLocator;
use crate::elf::ElfImage;
use crate::plan::{AttachPoint, CaptureStrategy, PayloadDirection, ProbePoint};
use crate::probe_detector::contract::detection::{DetectionEvidence, EvidenceFact};

const DROP: &[u8] = &[
    0xfd, 0x7b, 0xbb, 0xa9, 0xf9, 0x0b, 0x00, 0xf9, 0xf8, 0x5f, 0x02, 0xa9, 0xf6, 0x57, 0x03, 0xa9,
    0xf4, 0x4f, 0x04, 0xa9, 0xfd, 0x03, 0x00, 0x91, 0x15, 0x50, 0x41, 0xa9, 0xf3, 0x03, 0x00, 0xaa,
    0x88, 0x02, 0x40, 0xf9, 0x68, 0x00, 0x00, 0xb4, 0xe0, 0x03, 0x15, 0xaa, 0x00, 0x01, 0x3f, 0xd6,
    0x88, 0x06, 0x40, 0xf9, 0x68, 0x00, 0x00, 0xb4, 0xe0, 0x03, 0x15, 0xaa,
];
const CERTIFICATES: &[u8] = &[
    0x68, 0x7a, 0x41, 0xf9, 0x1f, 0x01, 0x14, 0xeb, 0xa0, 0x02, 0x00, 0x54, 0x74, 0x82, 0x41, 0xf9,
    0x14, 0x02, 0x00, 0xb4, 0x68, 0x7e, 0x41, 0xf9, 0x16, 0x00, 0xf0, 0xd2, 0x15, 0x21, 0x00, 0x91,
    0x04, 0x00, 0x00, 0x14, 0x94, 0x06, 0x00, 0xf1, 0xb5, 0x62, 0x00, 0x91, 0x00, 0x01, 0x00, 0x54,
    0xa8, 0x82, 0x5f, 0xf8, 0x1f, 0x01, 0x16, 0xeb, 0x60, 0xff, 0xff, 0x54, 0x48, 0xff, 0xff, 0xb4,
    0xa0, 0x02, 0x40, 0xf9,
];
// Both inline and outlined RecordLayer destructors reach this Vec ownership check.
const CERTIFICATE_HEAD: &[u8] = &[0x68, 0x7a, 0x41, 0xf9, 0x1f, 0x01, 0x14, 0xeb];
const OUTLINED_DROP_PREFIX: &[u32] = &[
    0xa9bb7bfd, 0xf9000bf9, 0xa9025ff8, 0xa90357f6, 0xa9044ff4, 0x910003fd, 0xaa0003f3,
];
const OUTLINED_OPTION: &[u32] = &[
    0xf9416e61, 0xd2f00014, 0xeb14003f, 0x540000a0, 0xb4000081, 0xf9417260, 0x2a1f03e2,
];
const OUTLINED_CERTIFICATES: &[u32] = &[
    0xf9417a68, 0xeb14011f, 0x540002e0, 0xf9418274, 0xb40001f4, 0xf9417e68, 0x91002115, 0x14000004,
    0xf1000694, 0x910062b5, 0x54000100, 0xf85f82a1, 0xf100043f, 0x54ffff6b, 0xf94002a0, 0x2a1f03e2,
];
const OUTLINED_CERTIFICATE_FREE: &[u32] = &[
    0x17fffff7, 0xf9417a68, 0xb40000c8, 0x8b080508, 0xf9417e60, 0x2a1f03e2, 0xd37df101,
];
const RECORD_LAYER_DROP: &[u32] = &[
    0xa9bd7bfd, 0xf9000bf5, 0xa9024ff4, 0x910003fd, 0xa9415015, 0xaa0003f3, 0xf9400288, 0xb4000068,
    0xaa1503e0, 0xd63f0100, 0xf9400681, 0xb4000121, 0xf9400a88, 0xaa1503e0, 0xdac00109, 0xeb01011f,
    0xfa519902, 0xdac01128, 0x1a8833e2,
];
const RECORD_LAYER_SECOND_BOX: &[u32] = &[
    0xa9425673, 0xf94002a8, 0xb4000068, 0xaa1303e0, 0xd63f0100, 0xf94006a1,
];

pub(in crate::probe_detector::detector::tls) struct RustlsConnectionLayout;

impl RustlsConnectionLayout {
    pub(in crate::probe_detector::detector::tls::rustls) const BUFFER_48_PATTERN: &[u8] = &[
        0xfd, 0x7b, 0xbc, 0xa9, 0xf8, 0x5f, 0x01, 0xa9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03,
        0xa9, 0xfd, 0x03, 0x00, 0x91, 0x17, 0x84, 0x41, 0xf9, 0x08, 0x00, 0xf0, 0xd2, 0xf5, 0x03,
        0x02, 0xaa, 0xf3, 0x03, 0x01, 0xaa, 0xf4, 0x03, 0x00, 0xaa, 0x08, 0x84, 0x01, 0xf9, 0xff,
        0x02, 0x08, 0xeb,
    ];
    pub(in crate::probe_detector::detector::tls::rustls) const TAKE_56_PATTERN: &[u8] = &[
        0xfd, 0x7b, 0xbd, 0xa9, 0xf6, 0x57, 0x01, 0xa9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x03, 0x00,
        0x91, 0x36, 0x50, 0x40, 0xa9, 0x09, 0x00, 0xf0, 0xd2, 0x33, 0x08, 0x40, 0xf9, 0xf5, 0x03,
        0x00, 0xaa, 0x08, 0x04, 0x80, 0x52, 0x08, 0xb8, 0x0c, 0x39, 0xdf, 0x02, 0x09, 0xeb, 0x81,
        0x02, 0x00, 0x54, 0xd3, 0x00, 0xf8, 0xb6, 0xe0, 0x03, 0x1f, 0xaa,
    ];

    pub(in crate::probe_detector::detector::tls::rustls) const BUFFER_PATTERN: &[u8] = &[
        0xff, 0x83, 0x01, 0xd1, 0xfd, 0x7b, 0x02, 0xa9, 0xf8, 0x5f, 0x03, 0xa9, 0xf6, 0x57, 0x04,
        0xa9, 0xf4, 0x4f, 0x05, 0xa9, 0xfd, 0x83, 0x00, 0x91, 0x17, 0x84, 0x41, 0xf9, 0x08, 0x00,
        0xf0, 0xd2, 0xf4, 0x03, 0x02, 0xaa, 0xf3, 0x03, 0x01, 0xaa, 0xf5, 0x03, 0x00, 0xaa, 0x08,
        0x84, 0x01, 0xf9, 0xff, 0x02, 0x08, 0xeb,
    ];
    pub(in crate::probe_detector::detector::tls::rustls) const TAKE_PATTERN: &[u8] = &[
        0xfd, 0x7b, 0xbc, 0xa9, 0xf7, 0x0b, 0x00, 0xf9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03,
        0xa9, 0xfd, 0x03, 0x00, 0x91, 0x37, 0x50, 0x40, 0xa9, 0x09, 0x00, 0xf0, 0xd2, 0x33, 0x08,
        0x40, 0xf9, 0xf5, 0x03, 0x00, 0xaa, 0x08, 0x04, 0x80, 0x52, 0x08, 0xb8, 0x0c, 0x39, 0xff,
        0x02, 0x09, 0xeb, 0xa1, 0x00, 0x00, 0x54, 0x73, 0x01, 0xf8, 0xb6, 0xe0, 0x03, 0x1f, 0xaa,
        0xe1, 0x03, 0x13, 0xaa,
    ];

    pub(in crate::probe_detector::detector::tls::rustls) const BUFFER_X22_PATTERN: &[u8] = &[
        0xff, 0x83, 0x01, 0xd1, 0xfd, 0x7b, 0x02, 0xa9, 0xf8, 0x5f, 0x03, 0xa9, 0xf6, 0x57, 0x04,
        0xa9, 0xf4, 0x4f, 0x05, 0xa9, 0xfd, 0x83, 0x00, 0x91, 0x16, 0x84, 0x41, 0xf9, 0x08, 0x00,
        0xf0, 0xd2, 0xf4, 0x03, 0x02, 0xaa, 0xf3, 0x03, 0x01, 0xaa, 0xf5, 0x03, 0x00, 0xaa, 0x08,
        0x84, 0x01, 0xf9, 0xdf, 0x02, 0x08, 0xeb,
    ];
    pub(in crate::probe_detector::detector::tls::rustls) const TAKE_X22_PATTERN: &[u8] = &[
        0xfd, 0x7b, 0xbc, 0xa9, 0xf7, 0x0b, 0x00, 0xf9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03,
        0xa9, 0xfd, 0x03, 0x00, 0x91, 0x36, 0x50, 0x40, 0xa9, 0x09, 0x00, 0xf0, 0xd2, 0x33, 0x08,
        0x40, 0xf9, 0xf5, 0x03, 0x00, 0xaa, 0x08, 0x04, 0x80, 0x52, 0x08, 0xb8, 0x0c, 0x39, 0xdf,
        0x02, 0x09, 0xeb, 0xa1, 0x00, 0x00, 0x54, 0x73, 0x01, 0xf8, 0xb6, 0xe0, 0x03, 0x1f, 0xaa,
        0xe1, 0x03, 0x13, 0xaa,
    ];

    pub(super) fn register(image: &ElfImage) {
        if image.arch().as_str() == "aarch64" {
            image.register_pattern_scan(CERTIFICATE_HEAD);
        }
    }

    pub(super) fn augment(
        image: &ElfImage,
        locator: &RustlsHandshakeLocator,
        points: &mut Vec<ProbePoint>,
        evidence: &mut DetectionEvidence,
    ) -> Option<()> {
        let buffer = points
            .iter()
            .find(|point| point.symbol == "rustls_buffer_plaintext")?;
        // A structural handshake match alone does not prove certificate ownership.
        let verified_pair = [
            (Self::BUFFER_PATTERN, Self::TAKE_PATTERN),
            (Self::BUFFER_48_PATTERN, Self::TAKE_56_PATTERN),
            (Self::BUFFER_X22_PATTERN, Self::TAKE_X22_PATTERN),
        ]
        .into_iter()
        .any(|(buffer, take)| {
            points.len() == 2
                && points.iter().all(|point| {
                    let expected = match point.symbol.as_str() {
                        "rustls_buffer_plaintext" => buffer,
                        "rustls_take_received_plaintext" => take,
                        _ => return false,
                    };
                    usize::try_from(point.file_offset).ok().and_then(|offset| {
                        image
                            .data()
                            .get(offset..offset.checked_add(expected.len())?)
                    }) == Some(expected)
                })
        });
        if !verified_pair {
            return None;
        }
        let ranges = image.executable_file_ranges().ok()?;
        let offsets = image.pattern_offsets_for(&[CERTIFICATE_HEAD], &ranges);
        let certificates = Self::resolve_certificates(image, &offsets[0])?;
        let found = locator.locate(image, buffer.virtual_address).ok()?;
        let virtual_address = found.virtual_address;
        let offset = image
            .file_offset_for_virtual_address(virtual_address)
            .ok()?;
        for point in points.iter_mut() {
            point.cookie = certificates;
        }
        points.push(ProbePoint {
            symbol: "rustls_start_outgoing_traffic".to_string(),
            direction: PayloadDirection::Control,
            attach: AttachPoint::Entry,
            capture: CaptureStrategy::EntryBuffer,
            virtual_address,
            file_offset: offset,
            cookie: certificates,
        });
        evidence.facts.push(EvidenceFact {
            key: "rustls_handshake_structure".to_string(),
            value: format!(
                "entry=0x{:x};send-field={};caller-set-field={}",
                found.virtual_address, found.send_field, found.preceding_field
            ),
        });
        evidence.facts.push(EvidenceFact {
            key: "rustls_connection_identity".to_string(),
            value: format!("handshake-generation:certificate-vec-offset={certificates}"),
        });
        Some(())
    }

    fn resolve_certificates(image: &ElfImage, offsets: &[usize]) -> Option<u64> {
        let mut layout = None;
        for offset in offsets {
            let certificates = Self::certificate_offset(image, *offset);
            image.release_scan_window(*offset);
            let Some(certificates) = certificates else {
                continue;
            };
            if layout.is_some_and(|previous| previous != certificates) {
                return None;
            }
            layout = Some(certificates);
        }
        layout
    }

    fn certificate_offset(image: &ElfImage, offset: usize) -> Option<u64> {
        if Self::inline_layout(image, offset).is_none()
            && Self::outlined_layout(image, offset).is_none()
        {
            return None;
        }
        let bytes = image.data().get(offset..offset.checked_add(4)?)?;
        Some(u64::from((Self::word(bytes, 0)? >> 10) & 0xfff) * 8)
    }

    fn inline_layout(image: &ElfImage, certificates: usize) -> Option<()> {
        let start = certificates.checked_sub(128)?;
        let bytes = image.data().get(start..certificates.checked_add(68)?)?;
        // The two record-layer Box destructors precede the optional chain.
        (bytes.get(..DROP.len())? == DROP
            && Self::word(bytes, 60)? & 0xfc000000 == 0x94000000
            && bytes.get(64..96)?
                == &[
                    0x75, 0x52, 0x42, 0xa9, 0x88, 0x02, 0x40, 0xf9, 0x68, 0x00, 0x00, 0xb4, 0xe0,
                    0x03, 0x15, 0xaa, 0x00, 0x01, 0x3f, 0xd6, 0x88, 0x06, 0x40, 0xf9, 0x68, 0x00,
                    0x00, 0xb4, 0xe0, 0x03, 0x15, 0xaa,
                ]
            && Self::word(bytes, 96)? & 0xfc000000 == 0x94000000
            && bytes.get(128..196)? == CERTIFICATES)
            .then_some(())
    }

    fn outlined_layout(image: &ElfImage, certificates: usize) -> Option<()> {
        let start = certificates.checked_sub(64)?;
        let bytes = image.data().get(start..certificates.checked_add(104)?)?;
        if !(Self::matches_words(&bytes[..28], OUTLINED_DROP_PREFIX)
            && Self::word(bytes, 28)? & 0xfc000000 == 0x94000000
            && Self::matches_words(&bytes[32..60], OUTLINED_OPTION)
            && Self::word(bytes, 60)? & 0xfc000000 == 0x94000000
            && Self::matches_words(&bytes[64..128], OUTLINED_CERTIFICATES)
            && Self::word(bytes, 128)? & 0xfc000000 == 0x94000000
            && Self::matches_words(&bytes[132..160], OUTLINED_CERTIFICATE_FREE)
            && Self::word(bytes, 160)? & 0xfc000000 == 0x94000000
            && Self::word(bytes, 164)? == 0xf9403e75)
        {
            return None;
        }
        Self::record_layer_drop(image, start + 28, Self::word(bytes, 28)?)
    }

    fn record_layer_drop(image: &ElfImage, call_offset: usize, call: u32) -> Option<()> {
        let pc = image
            .virtual_address_for_file_offset(call_offset as u64)
            .ok()?;
        // BL carries a signed 26-bit word displacement.
        let displacement = i64::from((call << 6) as i32 >> 4);
        let target = pc.wrapping_add_signed(displacement);
        let offset = usize::try_from(image.file_offset_for_virtual_address(target).ok()?).ok()?;
        let bytes = image.data().get(offset..offset.checked_add(104)?)?;
        (Self::matches_words(&bytes[..76], RECORD_LAYER_DROP)
            && Self::word(bytes, 76)? & 0xfc000000 == 0x94000000
            && Self::matches_words(&bytes[80..104], RECORD_LAYER_SECOND_BOX))
        .then_some(())
    }

    fn matches_words(bytes: &[u8], words: &[u32]) -> bool {
        bytes
            .chunks_exact(4)
            .zip(words)
            .all(|(bytes, word)| bytes == word.to_le_bytes())
    }

    fn word(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
    }
}
