//! Static BoringSSL probe-point detection.

use std::collections::BTreeMap;

use crate::elf::{Arch, ElfImage};
use crate::{ToolError, ToolResult};

pub(crate) const NAME: &str = "boringssl";
pub(crate) const LIBRARY: &str = "boringssl";
pub(crate) const SYMBOLS: &[&str] = &[
    "SSL_do_handshake",
    "SSL_read",
    "SSL_read_ex",
    "SSL_write",
    "SSL_write_ex",
];
pub(crate) const MAP_SYMBOLS_X86_64: &[&str] = &["SSL_do_handshake", "SSL_read", "SSL_write"];
pub(crate) const MAP_SYMBOLS_AARCH64: &[&str] = &["SSL_read", "SSL_write"];
pub(crate) const SYMBOL_MAP_RESOLVER: &str = "bun-static-boringssl";
pub(crate) const STATIC_RESOLVER: &str = "boringssl-static";

const X86_64_HANDSHAKE_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec,
    0x28, 0x49, 0x89, 0xfc, 0x48, 0x8b, 0x47, 0x30,
];
const X86_64_READ_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x53, 0x50, 0x48, 0x83, 0xbf, 0x98, 0x00, 0x00,
    0x00, 0x00, 0x74,
];
const X86_64_WRITE_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec,
    0x18, 0x41, 0x89, 0xd7, 0x49, 0x89, 0xf6, 0x48, 0x89, 0xfb,
];
const X86_64_READ_HANDSHAKE_DELTA: usize = 0x6f0;
const X86_64_WRITE_READ_DELTA: usize = 0xca0;
const X86_64_WRITE_SEARCH_WINDOW: usize = 0x10000;

const AARCH64_READ_PATTERN: &[u8] = &[
    0xfd, 0x7b, 0xbd, 0xa9, 0xf5, 0x0b, 0x00, 0xf9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x03, 0x00, 0x91,
    0x08, 0x4c, 0x40, 0xf9, 0xa8, 0x01, 0x00, 0xb4,
];
const AARCH64_READ_INTERNAL_PATTERN: &[u8] = &[
    0xff, 0x03, 0x02, 0xd1, 0xfd, 0x7b, 0x04, 0xa9, 0xf8, 0x5f, 0x05, 0xa9, 0xf6, 0x57, 0x06, 0xa9,
    0xf4, 0x4f, 0x07, 0xa9, 0xfd, 0x03, 0x01, 0x91, 0x08, 0x18, 0x40, 0xf9, 0xf3, 0x03, 0x00, 0xaa,
];
const AARCH64_WRITE_PATTERN: &[u8] = &[
    0xff, 0x03, 0x01, 0xd1, 0xfd, 0x7b, 0x01, 0xa9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03, 0xa9,
    0xfd, 0x43, 0x00, 0x91, 0x08, 0x18, 0x40, 0xf9, 0xf5, 0x03, 0x02, 0x2a, 0xf4, 0x03, 0x01, 0xaa,
];
const AARCH64_WRITE_READ_DELTA: usize = 0x3c0;
const AARCH64_WRITE_READ_INTERNAL_DELTA: usize = 0x2c0;

pub(crate) struct StaticPatternDetection {
    pub(crate) arch_label: &'static str,
    pub(crate) matches: Vec<PatternMatches>,
    pub(crate) offsets: Vec<DetectedOffset>,
    pub(crate) map_symbols: BTreeMap<String, u64>,
}

pub(crate) struct PatternMatches {
    pub(crate) pattern_id: &'static str,
    pub(crate) symbol: &'static str,
    pub(crate) pattern_length: usize,
    pub(crate) match_count: usize,
    pub(crate) shown_matches: Vec<OffsetAddress>,
}

pub(crate) struct DetectedOffset {
    pub(crate) symbol: &'static str,
    pub(crate) file_offset: usize,
    pub(crate) virtual_address: u64,
}

pub(crate) struct OffsetAddress {
    pub(crate) file_offset: usize,
    pub(crate) virtual_address: u64,
}

pub(crate) fn map_symbols(arch: Arch) -> &'static [&'static str] {
    match arch {
        Arch::Aarch64 => MAP_SYMBOLS_AARCH64,
        Arch::X86_64 => MAP_SYMBOLS_X86_64,
    }
}

pub(crate) fn detect_static_patterns(
    image: &ElfImage,
    match_limit: usize,
) -> ToolResult<StaticPatternDetection> {
    match image.arch() {
        Arch::Aarch64 => detect_aarch64(image, match_limit),
        Arch::X86_64 => detect_x86_64(image, match_limit),
    }
}

fn detect_x86_64(image: &ElfImage, match_limit: usize) -> ToolResult<StaticPatternDetection> {
    let data = image.data();
    let handshake_matches = find_all(data, X86_64_HANDSHAKE_PATTERN);
    let read_matches = find_all(data, X86_64_READ_PATTERN);
    let write_matches = find_all(data, X86_64_WRITE_PATTERN);
    let read_offset = require_single(&read_matches, "SSL_read")?;
    let handshake_offset = resolve_x86_64_handshake(data, &handshake_matches, read_offset)?;
    let write_offset = resolve_x86_64_write(data, &write_matches, read_offset)?;
    let offsets = offsets_with_addresses(
        image,
        &[
            ("SSL_do_handshake", handshake_offset),
            ("SSL_read", read_offset),
            ("SSL_write", write_offset),
        ],
    )?;
    Ok(StaticPatternDetection {
        arch_label: "x86_64",
        matches: vec![
            pattern_matches(
                image,
                "x86_64-boringssl-ssl-do-handshake-entry-24",
                "SSL_do_handshake",
                X86_64_HANDSHAKE_PATTERN,
                &handshake_matches,
                match_limit,
            )?,
            pattern_matches(
                image,
                "x86_64-boringssl-ssl-read-entry-19",
                "SSL_read",
                X86_64_READ_PATTERN,
                &read_matches,
                match_limit,
            )?,
            pattern_matches(
                image,
                "x86_64-boringssl-ssl-write-entry-26",
                "SSL_write",
                X86_64_WRITE_PATTERN,
                &write_matches,
                match_limit,
            )?,
        ],
        map_symbols: map_from_offsets(&offsets, MAP_SYMBOLS_X86_64),
        offsets,
    })
}

fn detect_aarch64(image: &ElfImage, match_limit: usize) -> ToolResult<StaticPatternDetection> {
    let data = image.data();
    let read_matches = find_all(data, AARCH64_READ_PATTERN);
    let read_internal_matches = find_all(data, AARCH64_READ_INTERNAL_PATTERN);
    let write_matches = find_all(data, AARCH64_WRITE_PATTERN);
    let write_offset = require_single(&write_matches, "SSL_write")?;
    let read_offset = require_related(
        data,
        &read_matches,
        write_offset,
        AARCH64_READ_PATTERN,
        AARCH64_WRITE_READ_DELTA,
        "SSL_read",
    )?;
    let read_internal_offset = require_related(
        data,
        &read_internal_matches,
        write_offset,
        AARCH64_READ_INTERNAL_PATTERN,
        AARCH64_WRITE_READ_INTERNAL_DELTA,
        "SSL_read_internal",
    )?;
    let offsets = offsets_with_addresses(
        image,
        &[
            ("SSL_read", read_offset),
            ("SSL_read_internal", read_internal_offset),
            ("SSL_write", write_offset),
        ],
    )?;
    Ok(StaticPatternDetection {
        arch_label: "aarch64",
        matches: vec![
            pattern_matches(
                image,
                "arm64-boringssl-ssl-read-wrapper-24",
                "SSL_read",
                AARCH64_READ_PATTERN,
                &read_matches,
                match_limit,
            )?,
            pattern_matches(
                image,
                "arm64-boringssl-ssl-read-internal-32",
                "SSL_read_internal",
                AARCH64_READ_INTERNAL_PATTERN,
                &read_internal_matches,
                match_limit,
            )?,
            pattern_matches(
                image,
                "arm64-boringssl-ssl-write-entry-32",
                "SSL_write",
                AARCH64_WRITE_PATTERN,
                &write_matches,
                match_limit,
            )?,
        ],
        map_symbols: map_from_offsets(&offsets, MAP_SYMBOLS_AARCH64),
        offsets,
    })
}

fn resolve_x86_64_handshake(
    data: &[u8],
    handshake_matches: &[usize],
    read_offset: usize,
) -> ToolResult<usize> {
    if let Some(expected) = read_offset.checked_sub(X86_64_READ_HANDSHAKE_DELTA) {
        if matches_at(data, expected, X86_64_HANDSHAKE_PATTERN) {
            return Ok(expected);
        }
    }
    require_single(handshake_matches, "SSL_do_handshake")
}

fn resolve_x86_64_write(
    data: &[u8],
    write_matches: &[usize],
    read_offset: usize,
) -> ToolResult<usize> {
    let expected = read_offset
        .checked_add(X86_64_WRITE_READ_DELTA)
        .ok_or_else(|| ToolError::new("BoringSSL SSL_write expected offset overflow"))?;
    if matches_at(data, expected, X86_64_WRITE_PATTERN) {
        return Ok(expected);
    }
    let search_end = data.len().min(read_offset + X86_64_WRITE_SEARCH_WINDOW);
    let nearby = find_all(&data[read_offset..search_end], X86_64_WRITE_PATTERN)
        .into_iter()
        .map(|offset| read_offset + offset)
        .collect::<Vec<_>>();
    if nearby.len() == 1 {
        return Ok(nearby[0]);
    }
    if write_matches.len() == 1 {
        return Ok(write_matches[0]);
    }
    Err(ToolError::new(format!(
        "BoringSSL SSL_write nearby pattern match count={}",
        nearby.len()
    )))
}

fn require_related(
    data: &[u8],
    offsets: &[usize],
    write_offset: usize,
    pattern: &[u8],
    delta: usize,
    symbol: &str,
) -> ToolResult<usize> {
    let offset = require_single(offsets, symbol)?;
    let expected = write_offset.checked_sub(delta).ok_or_else(|| {
        ToolError::new(format!(
            "BoringSSL {symbol} offset underflows SSL_write delta"
        ))
    })?;
    if offset == expected && matches_at(data, expected, pattern) {
        Ok(offset)
    } else {
        Err(ToolError::new(format!(
            "BoringSSL {symbol} is not at SSL_write-0x{delta:x}"
        )))
    }
}

fn pattern_matches(
    image: &ElfImage,
    pattern_id: &'static str,
    symbol: &'static str,
    pattern: &[u8],
    offsets: &[usize],
    match_limit: usize,
) -> ToolResult<PatternMatches> {
    Ok(PatternMatches {
        pattern_id,
        symbol,
        pattern_length: pattern.len(),
        match_count: offsets.len(),
        shown_matches: offsets
            .iter()
            .copied()
            .take(match_limit)
            .map(|offset| offset_address(image, offset))
            .collect::<ToolResult<Vec<_>>>()?,
    })
}

fn offsets_with_addresses(
    image: &ElfImage,
    offsets: &[(&'static str, usize)],
) -> ToolResult<Vec<DetectedOffset>> {
    offsets
        .iter()
        .map(|(symbol, file_offset)| {
            Ok(DetectedOffset {
                symbol,
                file_offset: *file_offset,
                virtual_address: image.virtual_address_for_file_offset(*file_offset as u64)?,
            })
        })
        .collect()
}

fn offset_address(image: &ElfImage, file_offset: usize) -> ToolResult<OffsetAddress> {
    Ok(OffsetAddress {
        file_offset,
        virtual_address: image.virtual_address_for_file_offset(file_offset as u64)?,
    })
}

fn map_from_offsets(offsets: &[DetectedOffset], required: &[&str]) -> BTreeMap<String, u64> {
    required
        .iter()
        .filter_map(|symbol| {
            offsets
                .iter()
                .find(|offset| offset.symbol == *symbol)
                .map(|offset| ((*symbol).to_string(), offset.virtual_address))
        })
        .collect()
}

fn require_single(offsets: &[usize], symbol: &str) -> ToolResult<usize> {
    if offsets.len() == 1 {
        Ok(offsets[0])
    } else {
        Err(ToolError::new(format!(
            "BoringSSL {symbol} pattern match count={}",
            offsets.len()
        )))
    }
}

fn matches_at(data: &[u8], offset: usize, pattern: &[u8]) -> bool {
    data.get(offset..offset + pattern.len()) == Some(pattern)
}

fn find_all(data: &[u8], pattern: &[u8]) -> Vec<usize> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let mut offsets = Vec::new();
    let mut start = 0_usize;
    while start <= data.len().saturating_sub(pattern.len()) {
        let Some(relative) = data[start..].iter().position(|byte| *byte == pattern[0]) else {
            break;
        };
        let offset = start + relative;
        if data.get(offset..offset + pattern.len()) == Some(pattern) {
            offsets.push(offset);
        }
        start = offset + 1;
    }
    offsets
}
