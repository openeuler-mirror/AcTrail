//! Target process launch helpers for preloaded sync TLS runtime.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use tls_probe_point_finder::ProbePointPlan;

use crate::{SyncError, SyncResult};

const NATIVE_INLINE_HOOK_ARCHES: &[&str] = &["x86_64", "aarch64"];
const NATIVE_INLINE_HOOK_SYMBOLS: &[&str] = &[
    "SSL_write",
    "SSL_write_ex",
    "SSL_read",
    "SSL_read_ex",
    "rustls_buffer_plaintext",
    "rustls_take_received_plaintext",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeLibraryPath {
    Auto,
    Path(PathBuf),
}

pub fn run_with_preload(
    command: &[OsString],
    library: &Path,
    envs: Vec<(OsString, OsString)>,
) -> SyncResult<ExitStatus> {
    let Some(program) = command.first() else {
        return Err(SyncError::new("probe command is empty"));
    };
    let mut child = Command::new(program);
    child.args(&command[1..]);
    child.envs(envs);
    child.env("LD_PRELOAD", preload_env_value(library)?);
    child
        .status()
        .map_err(|error| SyncError::new(format!("run target: {error}")))
}

pub fn runtime_library_path(requested: &RuntimeLibraryPath) -> SyncResult<PathBuf> {
    match requested {
        RuntimeLibraryPath::Path(path) => return Ok(path.clone()),
        RuntimeLibraryPath::Auto => {}
    }
    if let Some(path) = std::env::var_os("TLS_PAYLOAD_SYNC_LIBRARY") {
        return Ok(PathBuf::from(path));
    }
    let executable = std::env::current_exe()
        .map_err(|error| SyncError::new(format!("resolve current executable: {error}")))?;
    let directory = executable
        .parent()
        .ok_or_else(|| SyncError::new("current executable has no parent directory"))?;
    let library = directory.join("libtls_payload_probe_sync.so");
    if !library.is_file() {
        return Err(SyncError::new(format!(
            "sync runtime library not found: {}",
            library.display()
        )));
    }
    Ok(library)
}

pub fn preload_env_value(library: &Path) -> SyncResult<OsString> {
    let library = std::fs::canonicalize(library).map_err(|error| {
        SyncError::new(format!(
            "cannot resolve sync runtime library {}: {error}",
            library.display()
        ))
    })?;
    let Some(existing) = std::env::var_os("LD_PRELOAD") else {
        return Ok(library.into_os_string());
    };
    let mut value = library.into_os_string();
    value.push(":");
    value.push(existing);
    Ok(value)
}

pub fn validate_native_backend_plan(plan: &ProbePointPlan) -> SyncResult<()> {
    let build_arch = std::env::consts::ARCH;
    if !NATIVE_INLINE_HOOK_ARCHES.contains(&build_arch) {
        return Err(SyncError::new(format!(
            "tls sync native backend was built for {build_arch}, but only x86_64 and aarch64 native inline hooks are implemented"
        )));
    }
    if plan.binary.architecture != build_arch {
        return Err(SyncError::new(format!(
            "tls sync native backend is built for {build_arch}, but probe binary is {}",
            plan.binary.architecture
        )));
    }
    if !plan.has_payload_closure() {
        return Err(SyncError::new("probe plan does not have payload closure"));
    }
    if let Some(point) = plan
        .points
        .iter()
        .find(|point| !is_native_inline_hook_symbol(&point.symbol))
    {
        let mut message = format!(
            "probe plan contains unsupported TLS sync native hook symbol: {}",
            point.symbol
        );
        if is_boringssl_validation_anchor(&point.symbol) {
            message.push_str(", which is a BoringSSL validation anchor rather than a payload hook");
        }
        return Err(SyncError::new(message));
    }
    Ok(())
}

fn is_native_inline_hook_symbol(symbol: &str) -> bool {
    NATIVE_INLINE_HOOK_SYMBOLS.contains(&symbol)
}

fn is_boringssl_validation_anchor(symbol: &str) -> bool {
    matches!(symbol, "SSL_do_handshake" | "SSL_read_internal")
}

pub fn launch_command_for_plan(
    command: &[OsString],
    plan: &ProbePointPlan,
) -> SyncResult<Vec<OsString>> {
    let Some(program) = command.first() else {
        return Err(SyncError::new("probe command is empty"));
    };
    let entry = resolve_command_path(program)?;
    let plan_binary = std::fs::canonicalize(&plan.binary.path).map_err(|error| {
        SyncError::new(format!(
            "cannot resolve probe binary {}: {error}",
            plan.binary.path.display()
        ))
    })?;
    if hidden_sibling_binary(&entry).is_some_and(|sibling| sibling == plan_binary) {
        let mut rewritten = command.to_vec();
        rewritten[0] = plan_binary.into_os_string();
        return Ok(rewritten);
    }
    Ok(command.to_vec())
}

fn resolve_command_path(program: &OsStr) -> SyncResult<PathBuf> {
    let raw = Path::new(program);
    if raw.components().count() > 1 {
        return std::fs::canonicalize(raw)
            .map_err(|error| SyncError::new(format!("cannot resolve {}: {error}", raw.display())));
    }
    let path_var = std::env::var_os("PATH").ok_or_else(|| SyncError::new("PATH is not set"))?;
    for directory in std::env::split_paths(&path_var) {
        let candidate = directory.join(raw);
        if candidate.is_file() {
            return std::fs::canonicalize(&candidate).map_err(|error| {
                SyncError::new(format!("cannot resolve {}: {error}", candidate.display()))
            });
        }
    }
    Err(SyncError::new(format!(
        "command not found on PATH: {}",
        program.to_string_lossy()
    )))
}

fn hidden_sibling_binary(entry: &Path) -> Option<PathBuf> {
    let parent = entry.parent()?;
    let file_name = entry.file_name()?;
    let mut hidden_name = OsString::from(".");
    hidden_name.push(file_name);
    std::fs::canonicalize(parent.join(hidden_name)).ok()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tls_probe_point_finder::{
        AttachPoint, CaptureStrategy, PayloadDirection, ProbeBinary, ProbePoint, ProbePointPlan,
        ProbeSource, TargetIdentity, TlsProvider,
    };

    use super::validate_native_backend_plan;

    #[test]
    fn native_backend_rejects_probe_binary_architecture_mismatch() {
        let probe_arch = match std::env::consts::ARCH {
            "x86_64" => "aarch64",
            _ => "x86_64",
        };
        let plan = plan(probe_arch, true);

        let error =
            validate_native_backend_plan(&plan).expect_err("architecture mismatch must fail");

        let message = error.to_string();
        assert!(message.contains("built for"));
        assert!(message.contains("probe binary"));
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn native_backend_rejects_incomplete_payload_closure() {
        let plan = plan(std::env::consts::ARCH, false);

        let error = validate_native_backend_plan(&plan).expect_err("incomplete plan must fail");

        assert!(error.to_string().contains("payload closure"));
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn native_backend_accepts_complete_payload_closure_on_supported_arch() {
        validate_native_backend_plan(&plan(std::env::consts::ARCH, true))
            .expect("supported native backend plan");
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn native_backend_rejects_boringssl_internal_validation_anchor() {
        let mut plan = plan(std::env::consts::ARCH, true);
        plan.points
            .push(point("SSL_read_internal", PayloadDirection::Inbound));

        let error = validate_native_backend_plan(&plan).expect_err("internal anchor must fail");

        assert!(error.to_string().contains("SSL_read_internal"));
        assert!(error.to_string().contains("validation anchor"));
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn native_backend_rejects_unsupported_validation_anchor() {
        let mut plan = plan(std::env::consts::ARCH, true);
        plan.points
            .push(point("SSL_do_handshake", PayloadDirection::Outbound));

        let error = validate_native_backend_plan(&plan).expect_err("handshake anchor must fail");

        assert!(error.to_string().contains("SSL_do_handshake"));
        assert!(error.to_string().contains("validation anchor"));
    }

    fn plan(architecture: &str, complete: bool) -> ProbePointPlan {
        let mut points = vec![point("SSL_read", PayloadDirection::Inbound)];
        if complete {
            points.push(point("SSL_write", PayloadDirection::Outbound));
        }
        ProbePointPlan {
            target: TargetIdentity {
                binary: PathBuf::from("/tmp/target"),
                architecture: architecture.to_string(),
                build_id: None,
            },
            provider: TlsProvider::OpenSsl,
            source: ProbeSource::Executable,
            resolver: "test".to_string(),
            binary: ProbeBinary {
                path: PathBuf::from("/tmp/target"),
                architecture: architecture.to_string(),
                build_id: None,
            },
            points,
        }
    }

    fn point(symbol: &str, direction: PayloadDirection) -> ProbePoint {
        ProbePoint {
            symbol: symbol.to_string(),
            direction,
            attach: AttachPoint::Entry,
            capture: CaptureStrategy::EntryBuffer,
            virtual_address: 0x1000,
            file_offset: 0,
        }
    }
}
