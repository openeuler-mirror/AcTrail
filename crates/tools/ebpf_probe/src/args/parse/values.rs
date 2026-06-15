//! Typed value parsing for eBPF probe CLI flags.

use std::collections::BTreeMap;
use std::path::PathBuf;

use config_core::daemon::{
    DisabledOrPath, EnforcementBackend, EnforcementDecision, EnforcementMarkStrategy,
    EnforcementScope, MemlockRlimit, PayloadRedactionPolicy, PayloadSocketCaptureBackend,
    PayloadSocketSeccompSyscall, PayloadStdioStorageMode, PayloadTlsCaptureBackend,
    PayloadTlsLibrary, PayloadTlsLibraryPath, PayloadTlsResolver, PayloadTlsSeccompSyscall,
    PayloadTlsSource, PayloadTlsSyncRuntimeLibraryPath, SseDataPolicy,
};

use super::super::MmapWorkloadConfig;

pub(super) fn config_only(flags: &BTreeMap<String, String>) -> bool {
    flags.contains_key("--config") && flags.len() == 1
}

pub(super) fn parse_flags(
    args: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>, String> {
    let mut parsed = BTreeMap::new();
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        if !flag.starts_with("--") {
            return Err(format!("expected flag, got {flag}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if parsed.insert(flag.clone(), value).is_some() {
            return Err(format!("duplicate flag {flag}"));
        }
    }
    Ok(parsed)
}

pub(super) fn optional_mmap_config(
    flags: &BTreeMap<String, String>,
) -> Result<Option<MmapWorkloadConfig>, String> {
    const MMAP_FLAGS: [&str; 4] = [
        "--mmap-path",
        "--mmap-message",
        "--mmap-length",
        "--mmap-offset",
    ];
    let present = MMAP_FLAGS
        .iter()
        .filter(|flag| flags.contains_key(**flag))
        .count();
    if present == 0 {
        return Ok(None);
    }
    if present != MMAP_FLAGS.len() {
        return Err(format!(
            "mmap workload flags must be provided together: {}",
            MMAP_FLAGS.join(", ")
        ));
    }
    Ok(Some(MmapWorkloadConfig {
        path: required_path(flags, "--mmap-path")?,
        message: required(flags, "--mmap-message")?,
        length: required_u64(flags, "--mmap-length")?,
        offset: required_u64(flags, "--mmap-offset")?,
    }))
}

pub(super) fn required(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<String, String> {
    flags
        .get(flag)
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required flag {flag}"))
}

pub(super) fn required_path(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PathBuf, String> {
    required(flags, flag).map(PathBuf::from)
}

pub(super) fn required_u64(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<u64, String> {
    required(flags, flag)?
        .parse::<u64>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_disabled_or_u64(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<Option<u64>, String> {
    let raw = required(flags, flag)?;
    if raw == "disabled" {
        return Ok(None);
    }
    let value = raw
        .parse::<u64>()
        .map_err(|error| format!("invalid {flag}: {error}"))?;
    if value == u64::default() {
        return Err(format!(
            "invalid {flag}: value must be positive or disabled"
        ));
    }
    Ok(Some(value))
}

pub(super) fn required_u32(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<u32, String> {
    required(flags, flag)?
        .parse::<u32>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_bool(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<bool, String> {
    match required(flags, flag)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        value => Err(format!(
            "invalid {flag}: expected true or false, got {value}"
        )),
    }
}

pub(super) fn optional_bool(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = flags.get(flag) else {
        return Ok(default);
    };
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        value => Err(format!(
            "invalid {flag}: expected true or false, got {value}"
        )),
    }
}

pub(super) fn required_octal_u32(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<u32, String> {
    u32::from_str_radix(&required(flags, flag)?, 8)
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_memlock_rlimit(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<MemlockRlimit, String> {
    required(flags, flag)?
        .parse::<MemlockRlimit>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_tls_library(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadTlsLibrary, String> {
    required(flags, flag)?
        .parse::<PayloadTlsLibrary>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_tls_source(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadTlsSource, String> {
    required(flags, flag)?
        .parse::<PayloadTlsSource>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_tls_resolver(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadTlsResolver, String> {
    required(flags, flag)?
        .parse::<PayloadTlsResolver>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_tls_capture_backend(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadTlsCaptureBackend, String> {
    required(flags, flag)?
        .parse::<PayloadTlsCaptureBackend>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_tls_seccomp_syscalls(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<Vec<PayloadTlsSeccompSyscall>, String> {
    required(flags, flag)?
        .split(',')
        .map(str::trim)
        .map(|value| {
            value
                .parse::<PayloadTlsSeccompSyscall>()
                .map_err(|error| format!("invalid {flag}: {error}"))
        })
        .collect()
}

pub(super) fn required_payload_socket_capture_backend(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadSocketCaptureBackend, String> {
    required(flags, flag)?
        .parse::<PayloadSocketCaptureBackend>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_socket_seccomp_syscalls(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<Vec<PayloadSocketSeccompSyscall>, String> {
    required(flags, flag)?
        .split(',')
        .map(str::trim)
        .map(|value| {
            value
                .parse::<PayloadSocketSeccompSyscall>()
                .map_err(|error| format!("invalid {flag}: {error}"))
        })
        .collect()
}

pub(super) fn required_payload_tls_library_path(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadTlsLibraryPath, String> {
    required(flags, flag)?
        .parse::<PayloadTlsLibraryPath>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_tls_sync_runtime_library_path(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadTlsSyncRuntimeLibraryPath, String> {
    required(flags, flag)?
        .parse::<PayloadTlsSyncRuntimeLibraryPath>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_disabled_or_path(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<DisabledOrPath, String> {
    required(flags, flag)?
        .parse::<DisabledOrPath>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_redaction_policy(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadRedactionPolicy, String> {
    required(flags, flag)?
        .parse::<PayloadRedactionPolicy>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_payload_stdio_storage_mode(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<PayloadStdioStorageMode, String> {
    required(flags, flag)?
        .parse::<PayloadStdioStorageMode>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_sse_data_policy(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<SseDataPolicy, String> {
    required(flags, flag)?
        .parse::<SseDataPolicy>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_enforcement_backend(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<EnforcementBackend, String> {
    required(flags, flag)?
        .parse::<EnforcementBackend>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_enforcement_scope(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<EnforcementScope, String> {
    required(flags, flag)?
        .parse::<EnforcementScope>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_enforcement_decision(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<EnforcementDecision, String> {
    required(flags, flag)?
        .parse::<EnforcementDecision>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

pub(super) fn required_enforcement_mark_strategy(
    flags: &BTreeMap<String, String>,
    flag: &'static str,
) -> Result<EnforcementMarkStrategy, String> {
    required(flags, flag)?
        .parse::<EnforcementMarkStrategy>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}
