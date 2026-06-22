//! Rustls probe-point provider metadata.

use std::collections::BTreeMap;

use crate::elf::{Arch, ElfImage};
use crate::{ToolError, ToolResult};

pub(crate) const NAME: &str = "rustls";
pub(crate) const LIBRARY: &str = "rustls";
pub(crate) const RESOLVER: &str = "rustls-symbol-map";
pub(crate) const RUNTIME_BUFFER_PLAINTEXT_SYMBOL: &str = "rustls_buffer_plaintext";
pub(crate) const RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL: &str = "rustls_take_received_plaintext";
pub(crate) const RUNTIME_SYMBOLS: &[&str] = &[
    RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
    RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
];

const DEMANGLED_BUFFER_PLAINTEXT_SUFFIX: &str =
    "rustls::common_state::CommonState::buffer_plaintext";
const DEMANGLED_TAKE_RECEIVED_PLAINTEXT_SUFFIX: &str =
    "rustls::common_state::CommonState::take_received_plaintext";

const X86_64_BUFFER_PLAINTEXT_PATTERN: &[u8] = &[
    0x55, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec, 0x28, 0x49, 0x89,
    0xd6, 0x48, 0x89, 0xf3, 0x4c, 0x8b, 0xa7, 0x08, 0x03, 0x00, 0x00,
];
const X86_64_BUFFER_PLAINTEXT_RUSTLS_0_23_40_PATTERN: &[u8] = &[
    0x55, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x50, 0x49, 0x89, 0xd6, 0x48, 0x89,
    0xf3, 0x4c, 0x8b, 0xa7, 0x08, 0x03, 0x00, 0x00, 0x48, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x80, 0x48, 0x89, 0x87, 0x08, 0x03, 0x00, 0x00,
];
const X86_64_TAKE_RECEIVED_PLAINTEXT_PATTERN: &[u8] = &[
    0x41, 0x57, 0x41, 0x56, 0x41, 0x54, 0x53, 0x50, 0x49, 0x89, 0xff, 0xc6, 0x87, 0x2e, 0x03, 0x00,
    0x00, 0x20, 0x4c, 0x8b, 0x26, 0x4c, 0x8b, 0x76, 0x08, 0x4c, 0x89, 0xe0, 0x48, 0xf7, 0xd8, 0x48,
];
const AARCH64_BUFFER_PLAINTEXT_RUSTLS_0_23_40_PATTERN: &[u8] = &[
    0xff, 0x83, 0x01, 0xd1, 0xfd, 0x7b, 0x02, 0xa9, 0xf8, 0x5f, 0x03, 0xa9, 0xf6, 0x57, 0x04, 0xa9,
    0xf4, 0x4f, 0x05, 0xa9, 0xfd, 0x83, 0x00, 0x91, 0x17, 0x84, 0x41, 0xf9, 0x08, 0x00, 0xf0, 0xd2,
    0xf4, 0x03, 0x02, 0xaa, 0xf3, 0x03, 0x01, 0xaa, 0xf5, 0x03, 0x00, 0xaa, 0x08, 0x84, 0x01, 0xf9,
    0xff, 0x02, 0x08, 0xeb,
];
const AARCH64_TAKE_RECEIVED_PLAINTEXT_RUSTLS_0_23_40_PATTERN: &[u8] = &[
    0xfd, 0x7b, 0xbc, 0xa9, 0xf7, 0x0b, 0x00, 0xf9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03, 0xa9,
    0xfd, 0x03, 0x00, 0x91, 0x37, 0x50, 0x40, 0xa9, 0x09, 0x00, 0xf0, 0xd2, 0x33, 0x08, 0x40, 0xf9,
    0xf5, 0x03, 0x00, 0xaa, 0x08, 0x04, 0x80, 0x52, 0x08, 0xb8, 0x0c, 0x39, 0xff, 0x02, 0x09, 0xeb,
    0xa1, 0x00, 0x00, 0x54, 0x73, 0x01, 0xf8, 0xb6, 0xe0, 0x03, 0x1f, 0xaa, 0xe1, 0x03, 0x13, 0xaa,
];

struct StaticPattern {
    pattern_id: &'static str,
    symbol: &'static str,
    bytes: &'static [u8],
}

struct StaticPatternSet {
    arch_label: &'static str,
    patterns: &'static [StaticPattern],
}

// Static machine-code signatures are architecture-specific. Keep each table
// isolated and select exactly one table from the target ELF architecture.
const X86_64_STATIC_PATTERNS: &[StaticPattern] = &[
    StaticPattern {
        pattern_id: "x86_64-rustls-common-state-buffer-plaintext-entry-27",
        symbol: RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
        bytes: X86_64_BUFFER_PLAINTEXT_PATTERN,
    },
    StaticPattern {
        pattern_id: "x86_64-rustls-0.23.40-common-state-buffer-plaintext-entry-41",
        symbol: RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
        bytes: X86_64_BUFFER_PLAINTEXT_RUSTLS_0_23_40_PATTERN,
    },
    StaticPattern {
        pattern_id: "x86_64-rustls-common-state-take-received-plaintext-entry-32",
        symbol: RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
        bytes: X86_64_TAKE_RECEIVED_PLAINTEXT_PATTERN,
    },
];
const AARCH64_STATIC_PATTERNS: &[StaticPattern] = &[
    StaticPattern {
        pattern_id: "aarch64-rustls-0.23.40-common-state-buffer-plaintext-entry-52",
        symbol: RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
        bytes: AARCH64_BUFFER_PLAINTEXT_RUSTLS_0_23_40_PATTERN,
    },
    StaticPattern {
        pattern_id: "aarch64-rustls-0.23.40-common-state-take-received-plaintext-entry-64",
        symbol: RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
        bytes: AARCH64_TAKE_RECEIVED_PLAINTEXT_RUSTLS_0_23_40_PATTERN,
    },
];

pub(crate) struct DemangledPlaintextSymbols {
    pub(crate) runtime_symbols: BTreeMap<String, u64>,
    pub(crate) targets: Vec<DemangledPlaintextTarget>,
}

pub(crate) struct DemangledPlaintextTarget {
    pub(crate) runtime_symbol: &'static str,
    pub(crate) symbol: String,
    pub(crate) address: u64,
}

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

pub(crate) fn resolve_demangled_plaintext_symbols(
    image: &ElfImage,
) -> ToolResult<Option<DemangledPlaintextSymbols>> {
    let mut targets = BTreeMap::<&'static str, DemangledPlaintextTarget>::new();
    for symbol in image.defined_function_symbols()? {
        let demangled = rustc_demangle::demangle(&symbol.raw_name).to_string();
        if let Some(target) = parse_demangled_target(
            &demangled,
            symbol.value,
            DEMANGLED_BUFFER_PLAINTEXT_SUFFIX,
            RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
        )? {
            targets.insert(RUNTIME_BUFFER_PLAINTEXT_SYMBOL, target);
        } else if let Some(target) = parse_demangled_target(
            &demangled,
            symbol.value,
            DEMANGLED_TAKE_RECEIVED_PLAINTEXT_SUFFIX,
            RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
        )? {
            targets.insert(RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL, target);
        }
    }
    if RUNTIME_SYMBOLS
        .iter()
        .all(|symbol| targets.contains_key(symbol))
    {
        let runtime_symbols = RUNTIME_SYMBOLS
            .iter()
            .map(|symbol| {
                (
                    (*symbol).to_string(),
                    targets.get(symbol).expect("target checked above").address,
                )
            })
            .collect();
        let targets = RUNTIME_SYMBOLS
            .iter()
            .map(|symbol| targets.remove(symbol).expect("target checked above"))
            .collect();
        Ok(Some(DemangledPlaintextSymbols {
            runtime_symbols,
            targets,
        }))
    } else {
        Ok(None)
    }
}

pub(crate) fn detect_static_patterns(
    image: &ElfImage,
    match_limit: usize,
) -> ToolResult<StaticPatternDetection> {
    let pattern_set = static_pattern_set(image.arch());
    detect_static_pattern_set(image, pattern_set, match_limit)
}

fn detect_static_pattern_set(
    image: &ElfImage,
    pattern_set: StaticPatternSet,
    match_limit: usize,
) -> ToolResult<StaticPatternDetection> {
    let data = image.data();
    let mut matches = Vec::new();
    let mut offsets_by_symbol = BTreeMap::<&'static str, Vec<usize>>::new();
    for pattern in pattern_set.patterns {
        let pattern_offsets = find_all(data, pattern.bytes);
        offsets_by_symbol
            .entry(pattern.symbol)
            .or_default()
            .extend(pattern_offsets.iter().copied());
        matches.push(pattern_matches(
            image,
            pattern.pattern_id,
            pattern.symbol,
            pattern.bytes,
            &pattern_offsets,
            match_limit,
        )?);
    }
    let required_offsets = RUNTIME_SYMBOLS
        .iter()
        .map(|symbol| {
            let offsets = offsets_by_symbol.remove(symbol).unwrap_or_default();
            Ok((*symbol, require_single_unique(&offsets, symbol)?))
        })
        .collect::<ToolResult<Vec<_>>>()?;
    let offsets = offsets_with_addresses(image, &required_offsets)?;
    Ok(StaticPatternDetection {
        arch_label: pattern_set.arch_label,
        matches,
        map_symbols: map_from_offsets(&offsets),
        offsets,
    })
}

fn static_pattern_set(arch: Arch) -> StaticPatternSet {
    match arch {
        Arch::X86_64 => StaticPatternSet {
            arch_label: "x86_64",
            patterns: X86_64_STATIC_PATTERNS,
        },
        Arch::Aarch64 => StaticPatternSet {
            arch_label: "aarch64",
            patterns: AARCH64_STATIC_PATTERNS,
        },
    }
}

fn parse_demangled_target(
    symbol: &str,
    address: u64,
    suffix: &str,
    runtime_symbol: &'static str,
) -> ToolResult<Option<DemangledPlaintextTarget>> {
    let matched = match symbol.strip_prefix(suffix) {
        Some("") => true,
        Some(tail) => tail.starts_with("::h"),
        None => false,
    };
    if !matched {
        return Ok(None);
    }
    Ok(Some(DemangledPlaintextTarget {
        runtime_symbol,
        symbol: symbol.to_string(),
        address,
    }))
}

fn find_all(data: &[u8], pattern: &[u8]) -> Vec<usize> {
    if pattern.is_empty() || pattern.len() > data.len() {
        return Vec::new();
    }
    data.windows(pattern.len())
        .enumerate()
        .filter_map(|(index, window)| (window == pattern).then_some(index))
        .collect()
}

fn require_single(matches: &[usize], symbol: &str) -> ToolResult<usize> {
    if matches.len() == 1 {
        Ok(matches[0])
    } else {
        Err(ToolError::new(format!(
            "rustls {symbol} pattern match count={}",
            matches.len()
        )))
    }
}

fn require_single_unique(matches: &[usize], symbol: &str) -> ToolResult<usize> {
    let mut unique = matches.to_vec();
    unique.sort_unstable();
    unique.dedup();
    require_single(&unique, symbol)
}

fn offsets_with_addresses(
    image: &ElfImage,
    offsets: &[(&'static str, usize)],
) -> ToolResult<Vec<DetectedOffset>> {
    offsets
        .iter()
        .map(|(symbol, file_offset)| {
            let virtual_address = image.virtual_address_for_file_offset(*file_offset as u64)?;
            Ok(DetectedOffset {
                symbol,
                file_offset: *file_offset,
                virtual_address,
            })
        })
        .collect()
}

fn pattern_matches(
    image: &ElfImage,
    pattern_id: &'static str,
    symbol: &'static str,
    pattern: &[u8],
    matches: &[usize],
    match_limit: usize,
) -> ToolResult<PatternMatches> {
    let shown_matches = matches
        .iter()
        .take(match_limit)
        .map(|file_offset| {
            let virtual_address = image.virtual_address_for_file_offset(*file_offset as u64)?;
            Ok(OffsetAddress {
                file_offset: *file_offset,
                virtual_address,
            })
        })
        .collect::<ToolResult<Vec<_>>>()?;
    Ok(PatternMatches {
        pattern_id,
        symbol,
        pattern_length: pattern.len(),
        match_count: matches.len(),
        shown_matches,
    })
}

fn map_from_offsets(offsets: &[DetectedOffset]) -> BTreeMap<String, u64> {
    offsets
        .iter()
        .map(|offset| (offset.symbol.to_string(), offset.virtual_address))
        .collect()
}
