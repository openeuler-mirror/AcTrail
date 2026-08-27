use std::collections::{BTreeMap, BTreeSet};

use super::constants::*;
use super::image::{ElfImage, ElfSection};
use super::raw::{bounded, read_u8, read_u16, read_u32, read_u64, string_at};
use crate::{ToolError, ToolResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SymbolMatch {
    pub(crate) value: u64,
    pub(crate) size: u64,
    pub(crate) bind: &'static str,
    pub(crate) ndx: String,
    pub(crate) table: String,
    pub(crate) raw_name: String,
}

impl SymbolMatch {
    pub(crate) fn is_defined(&self) -> bool {
        self.ndx != "UND" && self.value != 0
    }
}

/// Lazily shared symbol scan results.
///
/// Detectors register the exact symbol names (plus the rustls demangle
/// predicate) they may need before the first lookup; the first query runs
/// one pass over the symbol tables and only materialises matching entries.
/// Later lookups register incrementally and only rescan missing names, so a
/// library image queried by several providers stays correct with small extra
/// passes.
pub(crate) struct SymbolScanCache {
    wanted: BTreeSet<String>,
    include_rustls: bool,
    scanned: Option<BTreeMap<String, Vec<SymbolMatch>>>,
    scanned_wanted: BTreeSet<String>,
    scanned_rustls: bool,
}

impl SymbolScanCache {
    pub(crate) fn new() -> Self {
        Self {
            wanted: BTreeSet::new(),
            include_rustls: false,
            scanned: None,
            scanned_wanted: BTreeSet::new(),
            scanned_rustls: false,
        }
    }

    pub(crate) fn register_name(&mut self, name: &str) {
        self.wanted.insert(name.to_string());
    }

    pub(crate) fn register_rustls(&mut self) {
        self.include_rustls = true;
    }

    pub(crate) fn scan_if_needed(
        &mut self,
        data: &[u8],
        sections: &[ElfSection],
    ) -> ToolResult<bool> {
        let missing = self
            .wanted
            .difference(&self.scanned_wanted)
            .cloned()
            .collect::<Vec<_>>();
        let rustls_missing = self.include_rustls && !self.scanned_rustls;
        if missing.is_empty() && !rustls_missing {
            return Ok(false);
        }
        let extra = parse_matching_symbols(data, sections, &missing, rustls_missing)?;
        let scanned = self.scanned.get_or_insert_with(BTreeMap::new);
        for (name, symbols) in extra {
            scanned.entry(name).or_default().extend(symbols);
        }
        self.scanned_wanted.extend(missing);
        if rustls_missing {
            self.scanned_rustls = true;
        }
        Ok(true)
    }

    pub(crate) fn matches_for(&self, name: &str) -> Vec<SymbolMatch> {
        self.scanned
            .as_ref()
            .and_then(|found| found.get(name))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn defined_symbols(&self) -> Vec<SymbolMatch> {
        self.scanned
            .as_ref()
            .map(|found| {
                found
                    .values()
                    .flatten()
                    .filter(|symbol| symbol.is_defined())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn parse_matching_symbols(
    data: &[u8],
    sections: &[ElfSection],
    wanted: &[String],
    include_rustls: bool,
) -> ToolResult<BTreeMap<String, Vec<SymbolMatch>>> {
    let wanted = wanted.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut found = BTreeMap::new();
    for section in sections.iter().filter(|section| {
        section.section_type == ELF_SECTION_DYNSYM || section.section_type == ELF_SECTION_SYMTAB
    }) {
        parse_symbol_table(data, sections, section, &wanted, include_rustls, &mut found)?;
    }
    Ok(found)
}

fn parse_symbol_table(
    data: &[u8],
    sections: &[ElfSection],
    section: &ElfSection,
    wanted: &BTreeSet<&str>,
    include_rustls: bool,
    found: &mut BTreeMap<String, Vec<SymbolMatch>>,
) -> ToolResult<()> {
    if section.entry_size == 0 {
        return Err(ToolError::new(format!(
            "ELF symbol table {} has zero entry size",
            table_name(section)
        )));
    }
    let strings = sections
        .get(section.link as usize)
        .ok_or_else(|| ToolError::new("ELF symbol string table is missing"))?;
    let strings = bounded(data, strings.file_offset, strings.size)?;
    let table = bounded(data, section.file_offset, section.size)?;
    // rustls 谓词是子串匹配：string table 里若没有 "rustls"，任何符号名都
    // 不可能命中，直接跳过每符号的 contains 搜索。
    let rustls_present = !include_rustls || memchr::memmem::find(strings, b"rustls").is_some();
    let entry_size = usize::try_from(section.entry_size)
        .map_err(|_| ToolError::new("ELF symbol entry size overflow"))?;
    if entry_size < ELF_SYMBOL_TABLE_ENTRY_SIZE {
        return Err(ToolError::new("ELF symbol table entry is too small"));
    }
    for raw_symbol in table.chunks_exact(entry_size) {
        let info = read_u8(raw_symbol, ELF_SYMBOL_INFO_FIELD)?;
        if info & ELF_SYMBOL_TYPE_MASK != ELF_SYMBOL_TYPE_FUNC {
            continue;
        }
        let Some(raw_name) = string_at(strings, read_u32(raw_symbol, ELF_SYMBOL_NAME_FIELD)?)?
        else {
            continue;
        };
        if !wanted.contains(raw_name)
            && !(include_rustls && rustls_present && raw_name.contains("rustls"))
        {
            continue;
        }
        let section_index = read_u16(raw_symbol, ELF_SYMBOL_SECTION_INDEX_FIELD)?;
        let match_entry = SymbolMatch {
            value: read_u64(raw_symbol, ELF_SYMBOL_VALUE_FIELD)?,
            size: read_u64(raw_symbol, ELF_SYMBOL_SIZE_FIELD)?,
            bind: symbol_bind(info >> 4),
            ndx: section_label(section_index),
            table: table_name(section).to_string(),
            raw_name: raw_name.to_string(),
        };
        found
            .entry(match_entry.raw_name.clone())
            .or_default()
            .push(match_entry);
    }
    Ok(())
}

impl ElfImage {
    pub(crate) fn register_symbol_names(&self, names: &[&str]) {
        self.with_symbol_cache(|cache| {
            for name in names {
                cache.register_name(name);
            }
        });
    }

    pub(crate) fn register_rustls_symbols(&self) {
        self.with_symbol_cache(SymbolScanCache::register_rustls);
    }

    pub(crate) fn defined_function_symbols(&self) -> ToolResult<Vec<SymbolMatch>> {
        let result = self.with_symbol_cache(|cache| {
            cache.register_rustls();
            let scanned = cache.scan_if_needed(&self.data, &self.sections)?;
            Ok((cache.defined_symbols(), scanned))
        });
        result.map(|(symbols, scanned)| {
            self.record_analysis(!scanned);
            symbols
        })
    }

    pub(crate) fn symbols_by_name(
        &self,
        names: &[String],
    ) -> ToolResult<BTreeMap<String, Vec<SymbolMatch>>> {
        let result = self.with_symbol_cache(|cache| {
            for name in names {
                cache.register_name(name);
            }
            let scanned = cache.scan_if_needed(&self.data, &self.sections)?;
            Ok((
                names
                    .iter()
                    .map(|name| (name.clone(), cache.matches_for(name)))
                    .collect(),
                scanned,
            ))
        });
        result.map(|(symbols, scanned)| {
            self.record_analysis(!scanned);
            symbols
        })
    }

    pub(crate) fn unique_defined_symbol_values(
        &self,
        names: &[&str],
    ) -> ToolResult<BTreeMap<String, u64>> {
        let owned = names
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>();
        let symbols = self.symbols_by_name(&owned)?;
        let mut resolved = BTreeMap::new();
        for name in names {
            let Some(matches) = symbols.get(*name) else {
                continue;
            };
            let addresses = matches
                .iter()
                .filter(|symbol| symbol.is_defined())
                .map(|symbol| symbol.value)
                .collect::<BTreeSet<_>>();
            if addresses.is_empty() {
                continue;
            }
            if addresses.len() != 1 {
                let formatted = matches
                    .iter()
                    .map(|symbol| format!("0x{:x}@{}", symbol.value, symbol.table))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(ToolError::new(format!(
                    "ELF symbol table has multiple {name} addresses: {formatted}"
                )));
            }
            resolved.insert(
                (*name).to_string(),
                *addresses.iter().next().expect("one address"),
            );
        }
        Ok(resolved)
    }

    fn with_symbol_cache<R>(&self, operation: impl FnOnce(&mut SymbolScanCache) -> R) -> R {
        if let Some(cache) = &self.analysis_cache {
            cache.with_symbols(&self.analysis_key, operation)
        } else {
            operation(&mut self.symbol_cache.borrow_mut())
        }
    }
}

fn table_name(section: &ElfSection) -> &str {
    if section.name.is_empty() {
        "unknown"
    } else {
        section.name.as_str()
    }
}

fn section_label(index: u16) -> String {
    if index == ELF_SECTION_UNDEFINED {
        "UND".to_string()
    } else {
        index.to_string()
    }
}

fn symbol_bind(bind: u8) -> &'static str {
    match bind {
        ELF_SYMBOL_BIND_LOCAL => "LOCAL",
        ELF_SYMBOL_BIND_GLOBAL => "GLOBAL",
        ELF_SYMBOL_BIND_WEAK => "WEAK",
        ELF_SYMBOL_BIND_LOOS => "LOOS",
        ELF_SYMBOL_BIND_HIOS => "HIOS",
        ELF_SYMBOL_BIND_LOPROC => "LOPROC",
        ELF_SYMBOL_BIND_HIPROC => "HIPROC",
        _ => "UNKNOWN",
    }
}
