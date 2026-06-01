//! Rustls executable symbol-map resolution.

use std::collections::BTreeMap;
use std::path::Path;

use crate::loader::LoaderError;

use super::symbol_map::{ExecutableSymbolMapSpec, resolve_executable_symbol_offsets};

const RUSTLS_SYMBOL_MAP_SPEC: ExecutableSymbolMapSpec = ExecutableSymbolMapSpec {
    resolver: "rustls-symbol-map",
    library: "rustls",
    label: "rustls",
};

pub(super) fn resolve_rustls_offsets(
    binary_path: &Path,
    symbol_map_path: &Path,
    required_symbols: &[&str],
) -> Result<BTreeMap<String, usize>, LoaderError> {
    resolve_executable_symbol_offsets(
        binary_path,
        symbol_map_path,
        required_symbols,
        RUSTLS_SYMBOL_MAP_SPEC,
    )
}
