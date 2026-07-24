//! Executable TLS symbol-map resolution.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::loader::LoaderError;
use tls_probe_point_finder::{BinaryIdentity, BinaryIdentityTypeCode};

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
    let target_identity = tls_probe_point_finder::elf_identity_from_bytes(&binary)
        .map_err(|error| LoaderError::new("payload_tls_binary_path", error.to_string()))?;
    let elf = ElfImage::parse(&binary)?;
    let symbol_map = fs::read_to_string(symbol_map_path)
        .map_err(|error| LoaderError::new("payload_tls_pattern_path", error.to_string()))?;
    let symbols = ExecutableSymbolMap::parse(&symbol_map, spec.label)?;
    symbols.validate(required_symbols, &target_identity, &spec)?;

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
    identity: BinaryIdentity,
    symbols: BTreeMap<String, u64>,
}

impl ExecutableSymbolMap {
    fn parse(raw: &str, label: &str) -> Result<Self, LoaderError> {
        let mut resolver = None;
        let mut library = None;
        let mut arch = None;
        let mut identity_type_code = None;
        let mut identity = None;
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
                "identity_type_code" => {
                    let code = value.parse::<u16>().map_err(|error| {
                        LoaderError::new(
                            "payload_tls_pattern_path",
                            format!("invalid {label} identity type code: {error}"),
                        )
                    })?;
                    identity_type_code =
                        Some(BinaryIdentityTypeCode::parse(code).map_err(|error| {
                            LoaderError::new("payload_tls_pattern_path", error.to_string())
                        })?);
                }
                "identity" => identity = Some(value.to_string()),
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
            identity: BinaryIdentity::try_new(
                identity_type_code.ok_or_else(|| {
                    LoaderError::new(
                        "payload_tls_pattern_path",
                        format!("missing {label} identity_type_code"),
                    )
                })?,
                identity.ok_or_else(|| {
                    LoaderError::new(
                        "payload_tls_pattern_path",
                        format!("missing {label} identity"),
                    )
                })?,
            )
            .map_err(|error| LoaderError::new("payload_tls_pattern_path", error.to_string()))?,
            symbols,
        })
    }

    fn validate(
        &self,
        required_symbols: &[&str],
        target_identity: &BinaryIdentity,
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
        if target_identity != &self.identity {
            return Err(LoaderError::new(
                "payload_tls_pattern_path",
                format!(
                    "{} symbol-map identity {}:{} does not match target identity {}:{}",
                    spec.label,
                    self.identity.identity_type_code.code(),
                    self.identity.identity,
                    target_identity.identity_type_code.code(),
                    target_identity.identity
                ),
            ));
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
