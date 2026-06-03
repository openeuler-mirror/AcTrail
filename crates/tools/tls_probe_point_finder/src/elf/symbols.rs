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

impl ElfImage {
    pub(crate) fn defined_function_symbols(&self) -> ToolResult<Vec<SymbolMatch>> {
        let mut symbols = Vec::new();
        for section in self.symbol_sections() {
            read_all_defined_functions(&self.data, &self.sections, section, &mut symbols)?;
        }
        Ok(symbols)
    }

    pub(crate) fn symbols_by_name(
        &self,
        names: &[String],
    ) -> ToolResult<BTreeMap<String, Vec<SymbolMatch>>> {
        let wanted = names.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let mut found = names
            .iter()
            .map(|name| (name.clone(), Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for section in self.symbol_sections() {
            read_symbol_table(&self.data, &self.sections, section, &wanted, &mut found)?;
        }
        Ok(found)
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

    fn symbol_sections(&self) -> impl Iterator<Item = &ElfSection> {
        self.sections.iter().filter(|section| {
            section.section_type == ELF_SECTION_DYNSYM || section.section_type == ELF_SECTION_SYMTAB
        })
    }
}

fn read_all_defined_functions(
    data: &[u8],
    sections: &[ElfSection],
    section: &ElfSection,
    symbols: &mut Vec<SymbolMatch>,
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
        let section_index = read_u16(raw_symbol, ELF_SYMBOL_SECTION_INDEX_FIELD)?;
        let match_entry = SymbolMatch {
            value: read_u64(raw_symbol, ELF_SYMBOL_VALUE_FIELD)?,
            size: read_u64(raw_symbol, ELF_SYMBOL_SIZE_FIELD)?,
            bind: symbol_bind(info >> 4),
            ndx: section_label(section_index),
            table: table_name(section).to_string(),
            raw_name: raw_name.to_string(),
        };
        if match_entry.is_defined() {
            symbols.push(match_entry);
        }
    }
    Ok(())
}

fn read_symbol_table(
    data: &[u8],
    sections: &[ElfSection],
    section: &ElfSection,
    wanted: &BTreeSet<&str>,
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
        if !wanted.contains(raw_name) {
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
        if let Some(matches) = found.get_mut(raw_name) {
            matches.push(match_entry);
        }
    }
    Ok(())
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
