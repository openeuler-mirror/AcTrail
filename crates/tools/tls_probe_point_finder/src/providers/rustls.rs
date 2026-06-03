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
const X86_64_TAKE_RECEIVED_PLAINTEXT_PATTERN: &[u8] = &[
    0x41, 0x57, 0x41, 0x56, 0x41, 0x54, 0x53, 0x50, 0x49, 0x89, 0xff, 0xc6, 0x87, 0x2e, 0x03, 0x00,
    0x00, 0x20, 0x4c, 0x8b, 0x26, 0x4c, 0x8b, 0x76, 0x08, 0x4c, 0x89, 0xe0, 0x48, 0xf7, 0xd8, 0x48,
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
    match image.arch() {
        Arch::X86_64 => detect_x86_64(image, match_limit),
        Arch::Aarch64 => Err(ToolError::new(
            "rustls aarch64 static pattern detection is not defined",
        )),
    }
}

fn detect_x86_64(image: &ElfImage, match_limit: usize) -> ToolResult<StaticPatternDetection> {
    let data = image.data();
    let buffer_matches = find_all(data, X86_64_BUFFER_PLAINTEXT_PATTERN);
    let take_matches = find_all(data, X86_64_TAKE_RECEIVED_PLAINTEXT_PATTERN);
    let offsets = offsets_with_addresses(
        image,
        &[
            (
                RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
                require_single(&buffer_matches, RUNTIME_BUFFER_PLAINTEXT_SYMBOL)?,
            ),
            (
                RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
                require_single(&take_matches, RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL)?,
            ),
        ],
    )?;
    Ok(StaticPatternDetection {
        arch_label: "x86_64",
        matches: vec![
            pattern_matches(
                image,
                "x86_64-rustls-common-state-buffer-plaintext-entry-27",
                RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
                X86_64_BUFFER_PLAINTEXT_PATTERN,
                &buffer_matches,
                match_limit,
            )?,
            pattern_matches(
                image,
                "x86_64-rustls-common-state-take-received-plaintext-entry-32",
                RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
                X86_64_TAKE_RECEIVED_PLAINTEXT_PATTERN,
                &take_matches,
                match_limit,
            )?,
        ],
        map_symbols: map_from_offsets(&offsets),
        offsets,
    })
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
