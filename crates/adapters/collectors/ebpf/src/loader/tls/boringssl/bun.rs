//! Bun executable BoringSSL symbol-map resolution.

use std::collections::BTreeMap;
use std::path::Path;

use crate::loader::LoaderError;

use super::super::symbol_map::{ExecutableSymbolMapSpec, resolve_executable_symbol_offsets};

const BUN_STATIC_BORINGSSL_SPEC: ExecutableSymbolMapSpec = ExecutableSymbolMapSpec {
    resolver: "bun-static-boringssl",
    library: "boringssl",
    label: "Bun BoringSSL",
};

pub(super) fn resolve_bun_static_boringssl_offsets(
    binary_path: &Path,
    symbol_map_path: &Path,
    required_symbols: &[&str],
) -> Result<BTreeMap<String, usize>, LoaderError> {
    resolve_executable_symbol_offsets(
        binary_path,
        symbol_map_path,
        required_symbols,
        BUN_STATIC_BORINGSSL_SPEC,
    )
}
