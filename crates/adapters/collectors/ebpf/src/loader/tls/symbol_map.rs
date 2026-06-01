//! Executable TLS symbol-map resolution.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::loader::LoaderError;

use super::elf::ElfImage;

pub(super) struct ExecutableSymbolMapSpec {
    pub(super) resolver: &'static str,
    pub(super) library: &'static str,
    pub(super) label: &'static str,
}

pub(super) fn resolve_executable_symbol_offsets(
    binary_path: &Path,
    symbol_map_path: &Path,
    required_symbols: &[&str],
    spec: ExecutableSymbolMapSpec,
) -> Result<BTreeMap<String, usize>, LoaderError> {
    let binary = fs::read(binary_path)
        .map_err(|error| LoaderError::new("payload_tls_binary_path", error.to_string()))?;
    let elf = ElfImage::parse(&binary)?;
    let symbol_map = fs::read_to_string(symbol_map_path)
        .map_err(|error| LoaderError::new("payload_tls_pattern_path", error.to_string()))?;
    let symbols = ExecutableSymbolMap::parse(&symbol_map, spec.label)?;
    symbols.validate(required_symbols, elf.build_id(), &spec)?;

    required_symbols
        .iter()
        .map(|symbol| {
            let virtual_address = symbols.symbols.get(*symbol).copied().ok_or_else(|| {
                LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("missing {} symbol {symbol}", spec.label),
                )
            })?;
            elf.executable_file_offset(virtual_address, "payload_tls_pattern_path", spec.label)
                .map(|offset| ((*symbol).to_string(), offset))
        })
        .collect()
}

struct ExecutableSymbolMap {
    resolver: String,
    library: String,
    arch: String,
    build_id: String,
    symbols: BTreeMap<String, u64>,
}

impl ExecutableSymbolMap {
    fn parse(raw: &str, label: &str) -> Result<Self, LoaderError> {
        let mut resolver = None;
        let mut library = None;
        let mut arch = None;
        let mut build_id = None;
        let mut symbols = BTreeMap::new();
        for (line_index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (key, value) = trimmed.split_once('=').ok_or_else(|| {
                LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("invalid {label} symbol-map line {}", line_index + 1),
                )
            })?;
            let key = key.trim();
            let value = value.trim();
            match key {
                "resolver" => resolver = Some(value.to_string()),
                "library" => library = Some(value.to_string()),
                "arch" => arch = Some(value.to_string()),
                "build_id" => build_id = Some(normalize_build_id(value, label)?),
                "symbol" => {
                    let (symbol, virtual_address) = parse_symbol(value, label)?;
                    if symbols.insert(symbol.clone(), virtual_address).is_some() {
                        return Err(LoaderError::new(
                            "payload_tls_pattern_path",
                            format!("duplicate {label} symbol {symbol}"),
                        ));
                    }
                }
                other => {
                    return Err(LoaderError::new(
                        "payload_tls_pattern_path",
                        format!("unknown {label} symbol-map key {other}"),
                    ));
                }
            }
        }
        Ok(Self {
            resolver: resolver.ok_or_else(|| {
                LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("missing {label} resolver"),
                )
            })?,
            library: library.ok_or_else(|| {
                LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("missing {label} library"),
                )
            })?,
            arch: arch.ok_or_else(|| {
                LoaderError::new("payload_tls_pattern_path", format!("missing {label} arch"))
            })?,
            build_id: build_id.ok_or_else(|| {
                LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("missing {label} build_id"),
                )
            })?,
            symbols,
        })
    }

    fn validate(
        &self,
        required_symbols: &[&str],
        target_build_id: Option<&str>,
        spec: &ExecutableSymbolMapSpec,
    ) -> Result<(), LoaderError> {
        if self.resolver != spec.resolver {
            return Err(LoaderError::new(
                "payload_tls_pattern_path",
                format!(
                    "{} symbol map requires resolver={}",
                    spec.label, spec.resolver
                ),
            ));
        }
        if self.library != spec.library {
            return Err(LoaderError::new(
                "payload_tls_pattern_path",
                format!(
                    "{} symbol map requires library={}",
                    spec.label, spec.library
                ),
            ));
        }
        if self.arch != std::env::consts::ARCH {
            return Err(LoaderError::new(
                "payload_tls_pattern_path",
                format!(
                    "{} symbol-map arch {} does not match current arch {}",
                    spec.label,
                    self.arch,
                    std::env::consts::ARCH
                ),
            ));
        }
        match target_build_id {
            Some(target) if target == self.build_id => {}
            Some(target) => {
                return Err(LoaderError::new(
                    "payload_tls_pattern_path",
                    format!(
                        "{} symbol-map build_id {} does not match target build_id {}",
                        spec.label, self.build_id, target
                    ),
                ));
            }
            None => {
                return Err(LoaderError::new(
                    "payload_tls_binary_path",
                    "target executable has no GNU build-id note",
                ));
            }
        }
        for symbol in required_symbols {
            if !self.symbols.contains_key(*symbol) {
                return Err(LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("missing {} symbol {symbol}", spec.label),
                ));
            }
        }
        Ok(())
    }
}

fn parse_symbol(value: &str, label: &str) -> Result<(String, u64), LoaderError> {
    let (symbol, address) = value.split_once('|').ok_or_else(|| {
        LoaderError::new(
            "payload_tls_pattern_path",
            format!("{label} symbol must use symbol|virtual-address"),
        )
    })?;
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(LoaderError::new(
            "payload_tls_pattern_path",
            format!("{label} symbol name must not be empty"),
        ));
    }
    Ok((symbol.to_string(), parse_hex_u64(address.trim(), label)?))
}

fn parse_hex_u64(value: &str, label: &str) -> Result<u64, LoaderError> {
    let normalized = value.strip_prefix("0x").unwrap_or(value);
    if normalized.is_empty() {
        return Err(LoaderError::new(
            "payload_tls_pattern_path",
            format!("{label} symbol address must not be empty"),
        ));
    }
    u64::from_str_radix(normalized, 16).map_err(|error| {
        LoaderError::new(
            "payload_tls_pattern_path",
            format!("invalid {label} symbol address: {error}"),
        )
    })
}

fn normalize_build_id(value: &str, label: &str) -> Result<String, LoaderError> {
    let normalized = value
        .chars()
        .filter(|character| !character.is_whitespace() && *character != ':')
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() % 2 != 0 {
        return Err(LoaderError::new(
            "payload_tls_pattern_path",
            format!("{label} build_id must contain complete hex bytes"),
        ));
    }
    if !normalized
        .chars()
        .all(|character| character.is_ascii_hexdigit())
    {
        return Err(LoaderError::new(
            "payload_tls_pattern_path",
            format!("{label} build_id must be hexadecimal"),
        ));
    }
    Ok(normalized)
}
