//! OpenSSL shared-library resolution.

use std::path::PathBuf;
use std::process::Command;

use config_core::daemon::{EbpfCollectorConfig, PayloadTlsLibraryPath};

use crate::loader::LoaderError;

pub(super) fn resolve_openssl_library_path(
    config: &EbpfCollectorConfig,
) -> Result<PathBuf, LoaderError> {
    match &config.payload_tls.library_path {
        PayloadTlsLibraryPath::Path(path) => {
            if path.exists() {
                Ok(path.clone())
            } else {
                Err(LoaderError::new(
                    "payload_tls_library_path",
                    format!(
                        "configured TLS library path does not exist: {}",
                        path.display()
                    ),
                ))
            }
        }
        PayloadTlsLibraryPath::Auto => resolve_openssl_library_path_from_ldconfig(),
    }
}

fn resolve_openssl_library_path_from_ldconfig() -> Result<PathBuf, LoaderError> {
    let output = Command::new("ldconfig")
        .arg("-p")
        .output()
        .map_err(|error| LoaderError::new("payload_tls_library_path", error.to_string()))?;
    if !output.status.success() {
        return Err(LoaderError::new(
            "payload_tls_library_path",
            format!("ldconfig -p exited with {}", output.status),
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| LoaderError::new("payload_tls_library_path", error.to_string()))?;
    stdout
        .lines()
        .filter(|line| line.contains("libssl.so"))
        .filter_map(|line| {
            line.rsplit_once("=>")
                .map(|(_, path)| PathBuf::from(path.trim()))
        })
        .find(|path| path.exists())
        .ok_or_else(|| {
            LoaderError::new(
                "payload_tls_library_path",
                "ldconfig did not return an existing libssl.so path",
            )
        })
}
