//! Target process launch helpers for preloaded sync TLS runtime.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use tls_probe_point_finder::ProbePointPlan;
use tls_probe_point_finder::ProbeSource;

use crate::{SyncError, SyncResult};

const NATIVE_INLINE_HOOK_ARCHES: &[&str] = &["x86_64", "aarch64"];
const NATIVE_INLINE_HOOK_SYMBOLS: &[&str] = &[
    "SSL_write",
    "SSL_write_ex",
    "SSL_write_ex2",
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
    run_with_preload_libraries(command, &[library.to_path_buf()], envs)
}

pub fn run_with_preload_libraries(
    command: &[OsString],
    libraries: &[PathBuf],
    envs: Vec<(OsString, OsString)>,
) -> SyncResult<ExitStatus> {
    run_with_runtime_libraries(command, libraries, libraries, envs)
}

pub fn run_with_runtime_libraries(
    command: &[OsString],
    preload_libraries: &[PathBuf],
    audit_libraries: &[PathBuf],
    envs: Vec<(OsString, OsString)>,
) -> SyncResult<ExitStatus> {
    let Some(program) = command.first() else {
        return Err(SyncError::new("probe command is empty"));
    };
    let mut child = Command::new(program);
    child.args(&command[1..]);
    child.envs(envs);
    child.env(
        "LD_PRELOAD",
        preload_env_value_for_libraries(preload_libraries)?,
    );
    if !audit_libraries.is_empty() {
        child.env("LD_AUDIT", audit_env_value_for_libraries(audit_libraries)?);
        if let Some((key, value)) = audit_bind_now_env(audit_libraries) {
            child.env(key, value);
        }
    }
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
    let library = directory.join("libactrail_tls_payload_probe_sync.so");
    if !library.is_file() {
        return Err(SyncError::new(format!(
            "sync runtime library not found: {}",
            library.display()
        )));
    }
    Ok(library)
}

pub fn preload_env_value(library: &Path) -> SyncResult<OsString> {
    preload_env_value_for_libraries(&[library.to_path_buf()])
}

pub fn audit_env_value(library: &Path) -> SyncResult<OsString> {
    audit_env_value_for_libraries(&[library.to_path_buf()])
}

pub fn preload_env_value_for_libraries(libraries: &[PathBuf]) -> SyncResult<OsString> {
    loader_env_value_for_libraries(libraries, "LD_PRELOAD")
}

pub fn audit_env_value_for_libraries(libraries: &[PathBuf]) -> SyncResult<OsString> {
    loader_env_value_for_libraries(libraries, "LD_AUDIT")
}

pub fn audit_bind_now_env(audit_libraries: &[PathBuf]) -> Option<(OsString, OsString)> {
    if audit_libraries.is_empty() {
        return None;
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 lazy audit binding can clobber x8, which carries hidden
        // structure-return storage for PLT calls such as CPython _PyStatus.
        Some((OsString::from("LD_BIND_NOW"), OsString::from("1")))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        None
    }
}

pub fn audit_libraries_for_plans(
    runtime_libraries: &[PathBuf],
    plans: &[ProbePointPlan],
) -> Vec<PathBuf> {
    if plans
        .iter()
        .any(|plan| plan.source == ProbeSource::SharedLibrary)
    {
        runtime_libraries.to_vec()
    } else {
        Vec::new()
    }
}

fn loader_env_value_for_libraries(libraries: &[PathBuf], env_name: &str) -> SyncResult<OsString> {
    let mut resolved = libraries
        .iter()
        .map(|library| {
            std::fs::canonicalize(library).map_err(|error| {
                SyncError::new(format!(
                    "cannot resolve preload library {}: {error}",
                    library.display()
                ))
            })
        })
        .collect::<SyncResult<Vec<_>>>()?;
    if resolved.is_empty() {
        return Err(SyncError::new("preload library list must not be empty"));
    }
    let mut value = resolved.remove(0).into_os_string();
    for library in resolved {
        value.push(":");
        value.push(library);
    }
    let Some(existing) = std::env::var_os(env_name) else {
        return Ok(value);
    };
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

    use super::{
        audit_env_value_for_libraries, preload_env_value_for_libraries,
        validate_native_backend_plan,
    };

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

    #[test]
    fn preload_env_value_preserves_dependency_order() {
        let first = std::fs::canonicalize("/bin/sh").expect("canonical /bin/sh");
        let second = std::fs::canonicalize("/bin/ls").expect("canonical /bin/ls");

        let value = preload_env_value_for_libraries(&[first.clone(), second.clone()])
            .expect("preload value")
            .to_string_lossy()
            .into_owned();

        assert!(
            value.starts_with(&format!("{}:{}", first.display(), second.display())),
            "{value}"
        );
    }

    #[test]
    fn audit_env_value_preserves_dependency_order() {
        let first = std::fs::canonicalize("/bin/sh").expect("canonical /bin/sh");
        let second = std::fs::canonicalize("/bin/ls").expect("canonical /bin/ls");

        let value = audit_env_value_for_libraries(&[first.clone(), second.clone()])
            .expect("audit value")
            .to_string_lossy()
            .into_owned();

        assert!(
            value.starts_with(&format!("{}:{}", first.display(), second.display())),
            "{value}"
        );
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
