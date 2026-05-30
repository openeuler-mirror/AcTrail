//! Static BoringSSL executable probe resolution.

#[path = "boringssl/aarch64.rs"]
mod aarch64;
#[path = "boringssl/bun.rs"]
mod bun;
#[path = "boringssl/pattern.rs"]
mod pattern;
#[path = "boringssl/x86_64.rs"]
mod x86_64;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::loader::LoaderError;

pub(super) fn find_pattern_offsets(
    binary_path: &Path,
    pattern_path: &Path,
    required_symbols: &[&str],
) -> Result<BTreeMap<String, usize>, LoaderError> {
    pattern::find_pattern_offsets(binary_path, pattern_path, required_symbols)
}

pub(super) fn resolve_bun_static_boringssl_offsets(
    binary_path: &Path,
    symbol_map_path: &Path,
    required_symbols: &[&str],
) -> Result<BTreeMap<String, usize>, LoaderError> {
    bun::resolve_bun_static_boringssl_offsets(binary_path, symbol_map_path, required_symbols)
}

pub(super) fn resolve_static_boringssl_offsets(
    binary_path: &Path,
    required_symbols: &[&str],
) -> Result<BTreeMap<String, usize>, LoaderError> {
    let binary = fs::read(binary_path)
        .map_err(|error| LoaderError::new("payload_tls_binary_path", error.to_string()))?;
    let detected = match std::env::consts::ARCH {
        "aarch64" => aarch64::detect(&binary)?,
        "x86_64" => x86_64::detect(&binary)?,
        arch => {
            return Err(LoaderError::new(
                "payload_tls_resolver",
                format!("boringssl-static resolver does not support arch {arch}"),
            ));
        }
    };
    required_symbols
        .iter()
        .map(|symbol| {
            detected
                .offset(symbol)
                .map(|offset| ((*symbol).to_string(), offset))
        })
        .collect()
}

pub(super) struct StaticBoringSslOffsets {
    pub(super) ssl_read: usize,
    pub(super) ssl_write: usize,
}

impl StaticBoringSslOffsets {
    fn offset(&self, symbol: &str) -> Result<usize, LoaderError> {
        match symbol {
            "SSL_read" => Ok(self.ssl_read),
            "SSL_write" => Ok(self.ssl_write),
            other => Err(LoaderError::new(
                "payload_tls_resolver",
                format!("boringssl-static resolver cannot provide {other}"),
            )),
        }
    }
}

fn find_all(data: &[u8], pattern: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut start = 0;
    while start <= data.len().saturating_sub(pattern.len()) {
        let Some(relative) = data[start..]
            .windows(pattern.len())
            .position(|window| window == pattern)
        else {
            break;
        };
        let offset = start + relative;
        offsets.push(offset);
        start = offset + 1;
    }
    offsets
}

fn require_single(offsets: Vec<usize>, symbol: &str) -> Result<usize, LoaderError> {
    if offsets.len() == 1 {
        Ok(offsets[0])
    } else {
        Err(LoaderError::new(
            "payload_tls_resolver",
            format!("BoringSSL {symbol} pattern match count={}", offsets.len()),
        ))
    }
}

fn matches_at(data: &[u8], offset: usize, pattern: &[u8]) -> bool {
    data.get(offset..offset + pattern.len()) == Some(pattern)
}
