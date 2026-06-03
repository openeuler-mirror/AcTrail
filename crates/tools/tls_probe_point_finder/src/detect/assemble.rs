//! Helpers that assemble provider findings into report data.

use std::collections::BTreeMap;

use crate::elf::{Arch, ElfImage};
use crate::providers::{boringssl, rustls};
use crate::{ToolError, ToolResult};

use super::report::{
    DetectedOffsetReport, ExportedSymbolEntry, ExportedSymbolReport, OffsetAddressReport,
    PatternMatchReport, PatternMatchesReport, SymbolMapReport,
};

pub(super) fn exported_symbols(
    image: &ElfImage,
    names: &[String],
) -> ToolResult<Vec<ExportedSymbolReport>> {
    let symbols = image.symbols_by_name(names)?;
    Ok(names
        .iter()
        .map(|name| {
            let entries = symbols
                .get(name)
                .into_iter()
                .flatten()
                .map(|symbol| ExportedSymbolEntry {
                    value: format!("0x{:x}", symbol.value),
                    size: format!("0x{:x}", symbol.size),
                    bind: symbol.bind.to_string(),
                    ndx: symbol.ndx.clone(),
                    table: symbol.table.clone(),
                    raw: symbol.raw_name.clone(),
                })
                .collect();
            ExportedSymbolReport {
                name: name.clone(),
                entries,
            }
        })
        .collect())
}

pub(super) fn symbol_map_report(
    resolver: &str,
    library: &str,
    arch: Arch,
    build_id: Option<&str>,
    symbols: &BTreeMap<String, u64>,
) -> ToolResult<SymbolMapReport> {
    let build_id = build_id.ok_or_else(|| ToolError::new("target has no GNU build-id note"))?;
    Ok(SymbolMapReport {
        resolver: resolver.to_string(),
        library: library.to_string(),
        arch: arch.as_str().to_string(),
        build_id: build_id.to_string(),
        symbols: symbols
            .iter()
            .map(|(symbol, address)| (symbol.clone(), format!("0x{address:x}")))
            .collect(),
    })
}

pub(super) fn pattern_matches_report(
    detection: &boringssl::StaticPatternDetection,
) -> PatternMatchesReport {
    PatternMatchesReport {
        arch: detection.arch_label.to_string(),
        entries: detection
            .matches
            .iter()
            .map(|entry| PatternMatchReport {
                pattern_id: entry.pattern_id.to_string(),
                symbol: entry.symbol.to_string(),
                library: boringssl::LIBRARY.to_string(),
                resolver: boringssl::STATIC_RESOLVER.to_string(),
                pattern_length: format!("0x{:x}", entry.pattern_length),
                match_count: entry.match_count,
                matches: entry
                    .shown_matches
                    .iter()
                    .map(|found| OffsetAddressReport {
                        file_offset: format!("0x{:x}", found.file_offset),
                        virtual_address: format!("0x{:x}", found.virtual_address),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub(super) fn detected_offsets_report(
    detection: &boringssl::StaticPatternDetection,
) -> Vec<DetectedOffsetReport> {
    detection
        .offsets
        .iter()
        .map(|offset| DetectedOffsetReport {
            symbol: offset.symbol.to_string(),
            file_offset: format!("0x{:x}", offset.file_offset),
            virtual_address: format!("0x{:x}", offset.virtual_address),
        })
        .collect()
}

pub(super) fn rustls_pattern_matches_report(
    detection: &rustls::StaticPatternDetection,
) -> PatternMatchesReport {
    PatternMatchesReport {
        arch: detection.arch_label.to_string(),
        entries: detection
            .matches
            .iter()
            .map(|entry| PatternMatchReport {
                pattern_id: entry.pattern_id.to_string(),
                symbol: entry.symbol.to_string(),
                library: rustls::LIBRARY.to_string(),
                resolver: rustls::RESOLVER.to_string(),
                pattern_length: format!("0x{:x}", entry.pattern_length),
                match_count: entry.match_count,
                matches: entry
                    .shown_matches
                    .iter()
                    .map(|found| OffsetAddressReport {
                        file_offset: format!("0x{:x}", found.file_offset),
                        virtual_address: format!("0x{:x}", found.virtual_address),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub(super) fn rustls_detected_offsets_report(
    detection: &rustls::StaticPatternDetection,
) -> Vec<DetectedOffsetReport> {
    detection
        .offsets
        .iter()
        .map(|offset| DetectedOffsetReport {
            symbol: offset.symbol.to_string(),
            file_offset: format!("0x{:x}", offset.file_offset),
            virtual_address: format!("0x{:x}", offset.virtual_address),
        })
        .collect()
}

pub(super) fn names_with_extra(base: &[&str], extra: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for name in base {
        push_unique(&mut names, (*name).to_string());
    }
    for name in extra {
        push_unique(&mut names, name.clone());
    }
    names
}

pub(super) fn missing_symbols(required: &[&str], symbols: &BTreeMap<String, u64>) -> Vec<String> {
    required
        .iter()
        .filter(|symbol| !symbols.contains_key(**symbol))
        .map(|symbol| (*symbol).to_string())
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}
