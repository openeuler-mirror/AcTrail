//! TLS payload uprobe attachment.

#[path = "tls/boringssl.rs"]
mod boringssl;
#[path = "tls/diagnostics.rs"]
mod diagnostics;
#[path = "tls/elf.rs"]
mod elf;
#[path = "tls/go/resolve.rs"]
mod go;
#[path = "tls/go/dynamic.rs"]
mod go_dynamic;
#[path = "tls/openssl.rs"]
mod openssl;
#[path = "tls/pending.rs"]
mod pending;
#[path = "tls/rustls.rs"]
mod rustls;
#[path = "tls/symbol_map.rs"]
mod symbol_map;
#[path = "tls/targets.rs"]
mod targets;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use config_core::daemon::{
    DisabledOrPath, PayloadTlsCaptureBackend, PayloadTlsConfig, PayloadTlsLibrary,
    PayloadTlsLibraryPath, PayloadTlsResolver, PayloadTlsSource,
};
use libbpf_rs::{Link, MapCore, MapFlags, MapHandle, Object, UprobeOpts};

use crate::loader::LoaderError;
use boringssl::{
    find_pattern_offsets, resolve_bun_static_boringssl_offsets, resolve_static_boringssl_offsets,
};
use elf::{resolve_executable_symbol_offsets, resolve_shared_library_symbol_offsets};
use openssl::resolve_openssl_library_path;
use rustls::resolve_rustls_offsets;
use targets::{
    BORINGSSL_UPROBE_TARGETS, GO_UPROBE_TARGETS, OPENSSL_UPROBE_TARGETS, RUSTLS_UPROBE_TARGETS,
    TlsUprobeTarget,
};

pub(super) use diagnostics::read_tls_payload_diagnostics;
pub use diagnostics::{TlsPayloadDiagnosticCounter, TlsPayloadDiagnostics};
pub(super) use go_dynamic::{GoTlsAttachOutcome, attach_programs as attach_go_tls_programs};
pub use pending::PendingTlsPayloadOp;
pub(super) use pending::lookup_pending_payload_op;

pub const TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES: u32 = 4_194_303;
pub const TLS_PAYLOAD_DIRECT_COPY_MIN_RING_BUFFER_BYTES: u32 = 8_388_608;

const TLS_LIBRARY_OPENSSL: u32 = 1;
const TLS_LIBRARY_BORINGSSL: u32 = 2;
const TLS_LIBRARY_RUSTLS: u32 = 3;
const TLS_LIBRARY_GO: u32 = 4;
const TLS_BACKEND_SECCOMP_USER_READ: u32 = 1;
const TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK: u32 = 2;

pub fn validate_payload_config(config: &PayloadTlsConfig) -> Result<(), LoaderError> {
    if !config.enabled {
        return Ok(());
    }
    validate_payload_backend_config(config)?;
    if config.capture_backend.is_sync() {
        return validate_sync_payload_config(config);
    }
    match (config.source, config.resolver, config.library) {
        (
            PayloadTlsSource::SharedLibrary,
            PayloadTlsResolver::OpensslSymbols,
            PayloadTlsLibrary::Openssl,
        ) => validate_disabled_executable_fields(config),
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::OpensslSymbols,
            PayloadTlsLibrary::Openssl,
        ) => validate_executable_symbols_config(config),
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::BoringsslPatterns,
            PayloadTlsLibrary::Boringssl,
        ) => validate_executable_pattern_config(config),
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::BunStaticBoringssl,
            PayloadTlsLibrary::Boringssl,
        ) => validate_executable_pattern_config(config),
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::BoringsslStatic,
            PayloadTlsLibrary::Boringssl,
        ) => validate_executable_static_boringssl_config(config),
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::RustlsSymbolMap,
            PayloadTlsLibrary::Rustls,
        ) => validate_executable_pattern_config(config),
        (PayloadTlsSource::Executable, PayloadTlsResolver::GoPclntab, PayloadTlsLibrary::Go) => {
            go::validate_config(config)
        }
        _ => Err(LoaderError::new(
            "payload_tls_config",
            "unsupported payload TLS source/resolver/library combination",
        )),
    }
}

fn validate_sync_payload_config(config: &PayloadTlsConfig) -> Result<(), LoaderError> {
    if !matches!(config.binary_path, DisabledOrPath::Disabled)
        || !matches!(config.pattern_path, DisabledOrPath::Disabled)
    {
        return Err(LoaderError::new(
            "payload_tls_config",
            "tls-sync auto plan requires payload_tls_binary_path=disabled and payload_tls_pattern_path=disabled",
        ));
    }
    Ok(())
}

pub fn configure_payload_tls_map(
    object: &Object,
    config: &PayloadTlsConfig,
) -> Result<(), LoaderError> {
    if !config.enabled {
        return Ok(());
    }
    let map = object
        .maps()
        .find(|map| map.name() == OsStr::new("payload_tls_config"))
        .ok_or_else(|| LoaderError::new("payload_tls_config", "payload_tls_config map is missing"))
        .and_then(|map| {
            MapHandle::try_from(&map)
                .map_err(|error| LoaderError::new("payload_tls_config", error.to_string()))
        })?;
    let key = 0_u32.to_ne_bytes();
    let mut value = Vec::with_capacity(std::mem::size_of::<u32>() * 4);
    let (library, backend) = if config.capture_backend.is_sync() {
        (TLS_LIBRARY_GO, TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK)
    } else {
        (
            payload_tls_library_id(config)?,
            payload_tls_backend_id(config)?,
        )
    };
    value.extend_from_slice(&library.to_ne_bytes());
    value.extend_from_slice(&backend.to_ne_bytes());
    value.extend_from_slice(&config.max_segment_bytes.to_ne_bytes());
    value.extend_from_slice(&(config.diagnostics_enabled as u32).to_ne_bytes());
    map.update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new("payload_tls_config", error.to_string()))
}

pub fn attach_payload_tls_programs(
    object: &mut Object,
    config: &PayloadTlsConfig,
) -> Result<Vec<(Link, String)>, LoaderError> {
    if !config.enabled || config.capture_backend.is_sync() {
        return Ok(Vec::new());
    }
    let attach_points = payload_tls_attach_points(config)?;
    let mut links = Vec::new();
    for target in attach_points {
        let program = object
            .progs_mut()
            .find(|program| program.name() == OsStr::new(target.program))
            .ok_or_else(|| {
                LoaderError::new(
                    "attach_payload_tls",
                    format!("BPF program {} is missing", target.program),
                )
            })?;
        let link = match &target.location {
            TlsAttachLocation::Offset { path, offset } => program.attach_uprobe_with_opts(
                -1,
                path,
                *offset,
                UprobeOpts {
                    retprobe: target.retprobe,
                    ..Default::default()
                },
            ),
        }
        .map_err(|error| {
            LoaderError::new(
                "attach_payload_tls",
                format!("attach {} to {}: {error}", target.program, target.label),
            )
        })?;
        links.push((link, format!("{}:{}", target.program, target.label)));
    }
    Ok(links)
}

pub fn is_payload_tls_program(program_name: &str) -> bool {
    program_name.starts_with("handle_ssl_")
        || program_name.starts_with("handle_rustls_")
        || program_name.starts_with("handle_go_tls_")
}

pub fn is_go_tls_program(program_name: &str) -> bool {
    program_name.starts_with("handle_go_tls_")
}

fn validate_disabled_executable_fields(config: &PayloadTlsConfig) -> Result<(), LoaderError> {
    if !matches!(config.binary_path, DisabledOrPath::Disabled)
        || !matches!(config.pattern_path, DisabledOrPath::Disabled)
    {
        return Err(LoaderError::new(
            "payload_tls_config",
            "shared-library TLS source requires payload_tls_binary_path=disabled and payload_tls_pattern_path=disabled",
        ));
    }
    Ok(())
}

fn validate_payload_backend_config(config: &PayloadTlsConfig) -> Result<(), LoaderError> {
    match config.capture_backend {
        PayloadTlsCaptureBackend::SeccompUserRead => Ok(()),
        PayloadTlsCaptureBackend::BpfCopySeccompFallback => {
            if config.max_segment_bytes > TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES {
                return Err(LoaderError::new(
                    "payload_tls_config",
                    format!(
                        "payload_tls_max_segment_bytes {} exceeds BPF direct-copy ABI maximum {}",
                        config.max_segment_bytes, TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES
                    ),
                ));
            }
            if config.ring_buffer_bytes < TLS_PAYLOAD_DIRECT_COPY_MIN_RING_BUFFER_BYTES {
                return Err(LoaderError::new(
                    "payload_tls_config",
                    format!(
                        "payload_tls_ring_buffer_bytes {} is too small for bpf-copy-seccomp-fallback; minimum is {}",
                        config.ring_buffer_bytes, TLS_PAYLOAD_DIRECT_COPY_MIN_RING_BUFFER_BYTES
                    ),
                ));
            }
            Ok(())
        }
        PayloadTlsCaptureBackend::TlsSync => Ok(()),
    }
}

fn validate_executable_symbols_config(config: &PayloadTlsConfig) -> Result<(), LoaderError> {
    validate_executable_library_path(config)?;
    if !matches!(config.pattern_path, DisabledOrPath::Disabled) {
        return Err(LoaderError::new(
            "payload_tls_pattern_path",
            "openssl-symbols resolver requires payload_tls_pattern_path=disabled",
        ));
    }
    require_path(&config.binary_path, "payload_tls_binary_path").map(|_| ())
}

fn validate_executable_pattern_config(config: &PayloadTlsConfig) -> Result<(), LoaderError> {
    validate_executable_library_path(config)?;
    require_path(&config.binary_path, "payload_tls_binary_path")?;
    require_path(&config.pattern_path, "payload_tls_pattern_path").map(|_| ())
}

fn validate_executable_static_boringssl_config(
    config: &PayloadTlsConfig,
) -> Result<(), LoaderError> {
    validate_executable_library_path(config)?;
    require_path(&config.binary_path, "payload_tls_binary_path")?;
    validate_disabled_pattern_path(config, "boringssl-static")
}

fn validate_executable_library_path(config: &PayloadTlsConfig) -> Result<(), LoaderError> {
    if matches!(config.library_path, PayloadTlsLibraryPath::Auto) {
        return Ok(());
    }
    Err(LoaderError::new(
        "payload_tls_library_path",
        "executable TLS source does not use payload_tls_library_path; set it to auto",
    ))
}

fn payload_tls_attach_points(
    config: &PayloadTlsConfig,
) -> Result<Vec<TlsAttachPoint>, LoaderError> {
    match (config.source, config.resolver, config.library) {
        (
            PayloadTlsSource::SharedLibrary,
            PayloadTlsResolver::OpensslSymbols,
            PayloadTlsLibrary::Openssl,
        ) => {
            let path = resolve_openssl_library_path(config)?;
            let offsets = resolve_shared_library_symbol_offsets(
                &path,
                &target_symbols(OPENSSL_UPROBE_TARGETS),
                "OpenSSL shared library",
            )?;
            offset_attach_points(
                &path,
                &offsets,
                OPENSSL_UPROBE_TARGETS,
                "OpenSSL shared library",
            )
        }
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::OpensslSymbols,
            PayloadTlsLibrary::Openssl,
        ) => {
            let binary_path =
                require_existing_path(&config.binary_path, "payload_tls_binary_path")?;
            let offsets = resolve_executable_symbol_offsets(
                &binary_path,
                &target_symbols(OPENSSL_UPROBE_TARGETS),
                "OpenSSL",
            )?;
            offset_attach_points(
                &binary_path,
                &offsets,
                OPENSSL_UPROBE_TARGETS,
                "OpenSSL symbol",
            )
        }
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::BoringsslPatterns,
            PayloadTlsLibrary::Boringssl,
        ) => {
            let binary_path =
                require_existing_path(&config.binary_path, "payload_tls_binary_path")?;
            let pattern_path =
                require_existing_path(&config.pattern_path, "payload_tls_pattern_path")?;
            let offsets = find_pattern_offsets(
                &binary_path,
                &pattern_path,
                &target_symbols(BORINGSSL_UPROBE_TARGETS),
            )?;
            offset_attach_points(
                &binary_path,
                &offsets,
                BORINGSSL_UPROBE_TARGETS,
                "BoringSSL pattern",
            )
        }
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::BunStaticBoringssl,
            PayloadTlsLibrary::Boringssl,
        ) => {
            let binary_path =
                require_existing_path(&config.binary_path, "payload_tls_binary_path")?;
            let symbol_map_path =
                require_existing_path(&config.pattern_path, "payload_tls_pattern_path")?;
            let offsets = resolve_bun_static_boringssl_offsets(
                &binary_path,
                &symbol_map_path,
                &target_symbols(BORINGSSL_UPROBE_TARGETS),
            )?;
            offset_attach_points(
                &binary_path,
                &offsets,
                BORINGSSL_UPROBE_TARGETS,
                "Bun BoringSSL",
            )
        }
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::BoringsslStatic,
            PayloadTlsLibrary::Boringssl,
        ) => {
            let binary_path =
                require_existing_path(&config.binary_path, "payload_tls_binary_path")?;
            let symbols = target_symbols(BORINGSSL_UPROBE_TARGETS);
            let offsets = resolve_static_boringssl_offsets(&binary_path, &symbols)?;
            offset_attach_points(
                &binary_path,
                &offsets,
                BORINGSSL_UPROBE_TARGETS,
                "BoringSSL static",
            )
        }
        (
            PayloadTlsSource::Executable,
            PayloadTlsResolver::RustlsSymbolMap,
            PayloadTlsLibrary::Rustls,
        ) => {
            let binary_path =
                require_existing_path(&config.binary_path, "payload_tls_binary_path")?;
            let symbol_map_path =
                require_existing_path(&config.pattern_path, "payload_tls_pattern_path")?;
            let offsets = resolve_rustls_offsets(
                &binary_path,
                &symbol_map_path,
                &target_symbols(RUSTLS_UPROBE_TARGETS),
            )?;
            offset_attach_points(&binary_path, &offsets, RUSTLS_UPROBE_TARGETS, "rustls")
        }
        (PayloadTlsSource::Executable, PayloadTlsResolver::GoPclntab, PayloadTlsLibrary::Go) => {
            let binary_path =
                require_existing_path(&config.binary_path, "payload_tls_binary_path")?;
            let offsets = go::resolve_offsets(&binary_path, &target_symbols(GO_UPROBE_TARGETS))?;
            offset_attach_points(&binary_path, &offsets, GO_UPROBE_TARGETS, "Go crypto/tls")
        }
        _ => Err(LoaderError::new(
            "payload_tls_config",
            "unsupported payload TLS attach resolver",
        )),
    }
}

fn target_symbols(targets: &[TlsUprobeTarget]) -> Vec<&'static str> {
    targets.iter().fold(Vec::new(), |mut symbols, target| {
        if !symbols.contains(&target.symbol) {
            symbols.push(target.symbol);
        }
        symbols
    })
}

fn offset_attach_points(
    path: &Path,
    offsets: &BTreeMap<String, usize>,
    targets: &[TlsUprobeTarget],
    label: &'static str,
) -> Result<Vec<TlsAttachPoint>, LoaderError> {
    targets
        .iter()
        .map(|target| {
            let offset = offsets.get(target.symbol).copied().ok_or_else(|| {
                LoaderError::new(
                    "payload_tls_pattern_path",
                    format!("missing {label} offset for {}", target.symbol),
                )
            })?;
            Ok(TlsAttachPoint {
                program: target.program,
                retprobe: target.retprobe,
                label: format!("{}:0x{offset:x}:{}", path.display(), target.symbol),
                location: TlsAttachLocation::Offset {
                    path: path.to_path_buf(),
                    offset,
                },
            })
        })
        .collect()
}

fn payload_tls_library_id(config: &PayloadTlsConfig) -> Result<u32, LoaderError> {
    match config.library {
        PayloadTlsLibrary::Auto => Err(LoaderError::new(
            "payload_tls_library",
            "payload_tls_library=auto is only supported by tls-sync auto plan",
        )),
        PayloadTlsLibrary::Openssl => Ok(TLS_LIBRARY_OPENSSL),
        PayloadTlsLibrary::Boringssl => Ok(TLS_LIBRARY_BORINGSSL),
        PayloadTlsLibrary::Rustls => Ok(TLS_LIBRARY_RUSTLS),
        PayloadTlsLibrary::Go => Ok(TLS_LIBRARY_GO),
    }
}

fn payload_tls_backend_id(config: &PayloadTlsConfig) -> Result<u32, LoaderError> {
    match config.capture_backend {
        PayloadTlsCaptureBackend::SeccompUserRead => Ok(TLS_BACKEND_SECCOMP_USER_READ),
        PayloadTlsCaptureBackend::BpfCopySeccompFallback => {
            Ok(TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK)
        }
        PayloadTlsCaptureBackend::TlsSync => Err(LoaderError::new(
            "payload_tls_config",
            "tls-sync backend is not an eBPF TLS backend",
        )),
    }
}

fn require_path<'a>(value: &'a DisabledOrPath, key: &'static str) -> Result<&'a Path, LoaderError> {
    match value {
        DisabledOrPath::Path(path) => Ok(path),
        DisabledOrPath::Disabled => Err(LoaderError::new(key, format!("{key} must be a path"))),
    }
}

fn require_existing_path(
    value: &DisabledOrPath,
    key: &'static str,
) -> Result<PathBuf, LoaderError> {
    let path = require_path(value, key)?;
    if path.exists() {
        Ok(path.to_path_buf())
    } else {
        Err(LoaderError::new(
            key,
            format!("configured path does not exist: {}", path.display()),
        ))
    }
}

fn validate_disabled_pattern_path(
    config: &PayloadTlsConfig,
    resolver: &str,
) -> Result<(), LoaderError> {
    if matches!(config.pattern_path, DisabledOrPath::Disabled) {
        Ok(())
    } else {
        Err(LoaderError::new(
            "payload_tls_pattern_path",
            format!("{resolver} resolver requires payload_tls_pattern_path=disabled"),
        ))
    }
}

struct TlsAttachPoint {
    program: &'static str,
    retprobe: bool,
    label: String,
    location: TlsAttachLocation,
}

enum TlsAttachLocation {
    Offset { path: PathBuf, offset: usize },
}
