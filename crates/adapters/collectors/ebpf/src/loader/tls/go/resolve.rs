//! Go crypto/tls executable probe resolution.

use std::collections::BTreeMap;
use std::path::Path;

use config_core::daemon::{
    DisabledOrPath, PayloadTlsCaptureBackend, PayloadTlsConfig, PayloadTlsLibraryPath,
};

use crate::loader::LoaderError;

pub(super) fn validate_config(config: &PayloadTlsConfig) -> Result<(), LoaderError> {
    if !matches!(config.library_path, PayloadTlsLibraryPath::Auto) {
        return Err(LoaderError::new(
            "payload_tls_library_path",
            "go-pclntab executable TLS source does not use payload_tls_library_path; set it to auto",
        ));
    }
    if !matches!(config.pattern_path, DisabledOrPath::Disabled) {
        return Err(LoaderError::new(
            "payload_tls_pattern_path",
            "go-pclntab resolver requires payload_tls_pattern_path=disabled",
        ));
    }
    if !matches!(config.binary_path, DisabledOrPath::Path(_)) {
        return Err(LoaderError::new(
            "payload_tls_binary_path",
            "payload_tls_binary_path must be a path",
        ));
    }
    if config.capture_backend != PayloadTlsCaptureBackend::BpfCopySeccompFallback {
        return Err(LoaderError::new(
            "payload_tls_capture_backend",
            "go-pclntab requires payload_tls_capture_backend=bpf-copy-seccomp-fallback",
        ));
    }
    Ok(())
}

pub(super) fn resolve_offsets(
    binary_path: &Path,
    required_symbols: &[&str],
) -> Result<BTreeMap<String, usize>, LoaderError> {
    tls_probe_point_finder::resolve_go_pclntab_file_offsets(binary_path, required_symbols)
        .map_err(|error| LoaderError::new("payload_tls_resolver", error.to_string()))?
        .into_iter()
        .map(|(symbol, offset)| {
            usize::try_from(offset)
                .map(|converted| (symbol, converted))
                .map_err(|error| {
                    LoaderError::new(
                        "payload_tls_resolver",
                        format!("Go crypto/tls offset overflow: {error}"),
                    )
                })
        })
        .collect()
}
