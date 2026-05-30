//! TLS executable byte-pattern resolution.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::loader::LoaderError;

pub(super) fn find_pattern_offsets(
    binary_path: &Path,
    pattern_path: &Path,
    required_symbols: &[&str],
) -> Result<BTreeMap<String, usize>, LoaderError> {
    let binary = fs::read(binary_path)
        .map_err(|error| LoaderError::new("payload_tls_binary_path", error.to_string()))?;
    let pattern_raw = fs::read_to_string(pattern_path)
        .map_err(|error| LoaderError::new("payload_tls_pattern_path", error.to_string()))?;
    let pattern_set = PatternSet::parse(&pattern_raw)?;
    pattern_set.validate(required_symbols)?;
    pattern_set
        .functions
        .iter()
        .map(|function| {
            find_unique_pattern(&binary, &function.pattern).map(|offset| {
                let mut entry = BTreeMap::new();
                entry.insert(function.symbol.clone(), offset);
                entry
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .try_fold(BTreeMap::new(), |mut acc, entry| {
            for (symbol, offset) in entry {
                if acc.insert(symbol.clone(), offset).is_some() {
                    return Err(LoaderError::new(
                        "payload_tls_pattern_path",
                        format!("duplicate pattern for {symbol}"),
                    ));
                }
            }
            Ok(acc)
        })
}

fn find_unique_pattern(data: &[u8], pattern: &[u8]) -> Result<usize, LoaderError> {
    if pattern.is_empty() || pattern.len() > data.len() {
        return Err(LoaderError::new(
            "payload_tls_pattern_path",
            "pattern cannot match target binary",
        ));
    }
    let mut found = None;
    for offset in 0..=(data.len() - pattern.len()) {
        if data[offset..offset + pattern.len()] == *pattern {
            if found.is_some() {
                return Err(LoaderError::new(
                    "payload_tls_pattern_path",
                    "pattern matched more than one binary offset",
                ));
            }
            found = Some(offset);
        }
    }
    found
        .ok_or_else(|| LoaderError::new("payload_tls_pattern_path", "pattern did not match binary"))
}

struct PatternSet {
    library: String,
    arch: String,
    functions: Vec<PatternFunction>,
}

impl PatternSet {
    fn parse(raw: &str) -> Result<Self, LoaderError> {
        let mut library = None;
        let mut arch = None;
        let mut functions = Vec::new();
        for (line_index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (key, value) = trimmed.split_once('=').ok_or_else(|| {
                LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("invalid pattern line {}", line_index + 1),
                )
            })?;
            let key = key.trim();
            let value = value.trim();
            match key {
                "library" => library = Some(value.to_string()),
                "arch" => arch = Some(value.to_string()),
                "function" => functions.push(PatternFunction::parse(value)?),
                "pattern_set_name" => {
                    if value.is_empty() {
                        return Err(LoaderError::new(
                            "payload_tls_pattern_path",
                            "pattern_set_name must not be empty",
                        ));
                    }
                }
                other => {
                    return Err(LoaderError::new(
                        "payload_tls_pattern_path",
                        format!("unknown pattern key {other}"),
                    ));
                }
            }
        }
        Ok(Self {
            library: library.ok_or_else(|| {
                LoaderError::new("payload_tls_pattern_path", "missing pattern library")
            })?,
            arch: arch.ok_or_else(|| {
                LoaderError::new("payload_tls_pattern_path", "missing pattern arch")
            })?,
            functions,
        })
    }

    fn validate(&self, required_symbols: &[&str]) -> Result<(), LoaderError> {
        if self.library != "boringssl" {
            return Err(LoaderError::new(
                "payload_tls_pattern_path",
                "BoringSSL pattern resolver requires library=boringssl",
            ));
        }
        if self.arch != std::env::consts::ARCH {
            return Err(LoaderError::new(
                "payload_tls_pattern_path",
                format!(
                    "pattern arch {} does not match current arch {}",
                    self.arch,
                    std::env::consts::ARCH
                ),
            ));
        }
        for symbol in required_symbols {
            let count = self
                .functions
                .iter()
                .filter(|function| function.symbol == *symbol)
                .count();
            if count != 1 {
                return Err(LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("expected exactly one pattern for {symbol}, found {count}"),
                ));
            }
        }
        Ok(())
    }
}

struct PatternFunction {
    symbol: String,
    pattern: Vec<u8>,
}

impl PatternFunction {
    fn parse(value: &str) -> Result<Self, LoaderError> {
        let (symbol, pattern_hex) = value.split_once('|').ok_or_else(|| {
            LoaderError::new(
                "payload_tls_pattern_path",
                "function pattern must use symbol|hex-bytes",
            )
        })?;
        let symbol = symbol.trim();
        if !matches!(symbol, "SSL_write" | "SSL_read") {
            return Err(LoaderError::new(
                "payload_tls_pattern_path",
                format!("unsupported BoringSSL pattern symbol {symbol}"),
            ));
        }
        Ok(Self {
            symbol: symbol.to_string(),
            pattern: parse_hex_bytes(pattern_hex.trim())?,
        })
    }
}

fn parse_hex_bytes(value: &str) -> Result<Vec<u8>, LoaderError> {
    let normalized = value
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    if normalized.is_empty() || normalized.len() % 2 != 0 {
        return Err(LoaderError::new(
            "payload_tls_pattern_path",
            "hex pattern must contain complete bytes",
        ));
    }
    (0..normalized.len())
        .step_by(2)
        .map(|offset| {
            u8::from_str_radix(&normalized[offset..offset + 2], 16).map_err(|error| {
                LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("invalid hex byte: {error}"),
                )
            })
        })
        .collect()
}
