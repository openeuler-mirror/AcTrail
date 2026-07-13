//! Target process launch helpers for preloaded sync TLS runtime.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use tls_probe_point_finder::ProbePointPlan;
use tls_probe_point_finder::ProbeSource;

use crate::env::{
    ENV_DEPENDENCY_GUARD_DIR, ENV_LIBRARY_PATH_PREFIX, ENV_LIBRARY_PATH_PREFIX_GLIBC,
    ENV_LIBRARY_PATH_PREFIX_MUSL, ENV_RUNTIME_GLIBC_LIBRARY, ENV_RUNTIME_MUSL_LIBRARY,
    ENV_SYSTEM_LIBRARY_DIRS, RUNTIME_GLIBC_LIBRARY_NAME, RUNTIME_MUSL_LIBRARY_NAME,
};
use crate::plan::RuntimePlanDescriptor;
use crate::{SyncError, SyncResult};

const LD_LIBRARY_PATH: &str = "LD_LIBRARY_PATH";
const LIBGCC_S: &str = "libgcc_s.so.1";
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeLibrarySet {
    pub glibc: PathBuf,
    pub musl: Option<PathBuf>,
}

impl RuntimeLibrarySet {
    pub fn library_for(&self, family: crate::LibcFamily) -> SyncResult<PathBuf> {
        match family {
            crate::LibcFamily::Glibc => Ok(self.glibc.clone()),
            crate::LibcFamily::Musl => self.musl.clone().ok_or_else(|| {
                SyncError::new(format!(
                    "musl TLS sync runtime library not found; build {RUNTIME_MUSL_LIBRARY_NAME} with scripts/build-tls-sync-runtimes.sh or set {ENV_RUNTIME_MUSL_LIBRARY}"
                ))
            }),
        }
    }
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
    if let Some((key, value)) = runtime_dependency_library_path_env(preload_libraries)? {
        child.env(key, value);
    }
    if let Some((key, value)) = runtime_dependency_library_path_prefix_env(preload_libraries)? {
        child.env(key, value);
    }
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
        RuntimeLibraryPath::Path(path) => return canonical_runtime_library(path),
        RuntimeLibraryPath::Auto => {}
    }
    if let Some(path) = std::env::var_os("TLS_PAYLOAD_SYNC_LIBRARY") {
        return canonical_runtime_library(&PathBuf::from(path));
    }
    let executable = std::env::current_exe()
        .map_err(|error| SyncError::new(format!("resolve current executable: {error}")))?;
    let directory = executable
        .parent()
        .ok_or_else(|| SyncError::new("current executable has no parent directory"))?;
    let library = directory.join(RUNTIME_GLIBC_LIBRARY_NAME);
    if !library.is_file() {
        return Err(SyncError::new(format!(
            "sync runtime library not found: {}",
            library.display()
        )));
    }
    canonical_runtime_library(&library)
}

pub fn runtime_library_set(requested: &RuntimeLibraryPath) -> SyncResult<RuntimeLibrarySet> {
    let glibc = runtime_library_path(requested)?;
    let musl = runtime_musl_library_path(&glibc)?;
    Ok(RuntimeLibrarySet { glibc, musl })
}

pub fn runtime_musl_library_path(glibc_library: &Path) -> SyncResult<Option<PathBuf>> {
    if let Some(path) = std::env::var_os(ENV_RUNTIME_MUSL_LIBRARY) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return canonical_runtime_library(&path).map(Some);
        }
        return Err(SyncError::new(format!(
            "{ENV_RUNTIME_MUSL_LIBRARY} is not a file: {}",
            path.display()
        )));
    }
    let Some(directory) = glibc_library.parent() else {
        return Ok(None);
    };
    let candidate = directory.join(RUNTIME_MUSL_LIBRARY_NAME);
    if candidate.is_file() {
        return canonical_runtime_library(&candidate).map(Some);
    }
    Ok(None)
}

fn canonical_runtime_library(path: &Path) -> SyncResult<PathBuf> {
    if !path.is_file() {
        return Err(SyncError::new(format!(
            "sync runtime library not found: {}",
            path.display()
        )));
    }
    std::fs::canonicalize(path).map_err(|error| {
        SyncError::new(format!(
            "cannot resolve sync runtime library {}: {error}",
            path.display()
        ))
    })
}

pub fn runtime_library_envs(libraries: &RuntimeLibrarySet) -> Vec<(OsString, OsString)> {
    let mut envs = vec![(
        OsString::from(ENV_RUNTIME_GLIBC_LIBRARY),
        libraries.glibc.as_os_str().to_os_string(),
    )];
    if let Some(musl) = &libraries.musl {
        envs.push((
            OsString::from(ENV_RUNTIME_MUSL_LIBRARY),
            musl.as_os_str().to_os_string(),
        ));
    }
    envs
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

pub fn runtime_dependency_library_path_env(
    libraries: &[PathBuf],
) -> SyncResult<Option<(OsString, OsString)>> {
    let report = runtime_dependency_report(libraries)?;
    if report.library_path_prefix.is_empty() {
        return Ok(None);
    }
    let mut value = join_paths(&report.library_path_prefix);
    let Some(existing) = std::env::var_os(LD_LIBRARY_PATH).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    value.push(":");
    value.push(existing);
    Ok(Some((OsString::from(LD_LIBRARY_PATH), value)))
}

pub fn runtime_dependency_library_path_prefix_env(
    libraries: &[PathBuf],
) -> SyncResult<Option<(OsString, OsString)>> {
    let report = runtime_dependency_report(libraries)?;
    if report.library_path_prefix.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        OsString::from(ENV_LIBRARY_PATH_PREFIX),
        join_paths(&report.library_path_prefix),
    )))
}

pub fn runtime_dependency_library_path_prefix_glibc_env(
    libraries: &[PathBuf],
) -> SyncResult<Option<(OsString, OsString)>> {
    let report = runtime_dependency_report(libraries)?;
    if report.library_path_prefix.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        OsString::from(ENV_LIBRARY_PATH_PREFIX_GLIBC),
        join_paths(&report.library_path_prefix),
    )))
}

pub fn runtime_dependency_library_path_prefix_musl_env(
    paths: &[PathBuf],
) -> Option<(OsString, OsString)> {
    if paths.is_empty() {
        return None;
    }
    Some((
        OsString::from(ENV_LIBRARY_PATH_PREFIX_MUSL),
        join_paths(paths),
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDependencyReport {
    pub guarded_libraries: Vec<PathBuf>,
    pub library_path_prefix: Vec<PathBuf>,
}

pub fn runtime_dependency_report(libraries: &[PathBuf]) -> SyncResult<RuntimeDependencyReport> {
    let guarded_dependencies = runtime_dependency_preload_libraries(libraries)?;
    let library_path_prefix = runtime_dependency_guard_directories(&guarded_dependencies)?;
    let guarded_libraries = guarded_dependencies
        .iter()
        .map(|library| library.source.clone())
        .collect();
    Ok(RuntimeDependencyReport {
        guarded_libraries,
        library_path_prefix,
    })
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeDependencyLibrary {
    source: PathBuf,
    loader_name: OsString,
}

fn runtime_dependency_preload_libraries(
    libraries: &[PathBuf],
) -> SyncResult<Vec<RuntimeDependencyLibrary>> {
    let mut guards = Vec::new();
    if runtime_libraries_need(libraries, LIBGCC_S)? {
        let libgcc = resolve_system_library(LIBGCC_S)?;
        guards.push(libgcc);
    }
    Ok(guards)
}

fn runtime_dependency_guard_directories(
    libraries: &[RuntimeDependencyLibrary],
) -> SyncResult<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for library in libraries {
        let directory = prepare_dependency_guard_directory(library)?;
        if !directories.iter().any(|existing| existing == &directory) {
            directories.push(directory);
        }
    }
    Ok(directories)
}

fn prepare_dependency_guard_directory(library: &RuntimeDependencyLibrary) -> SyncResult<PathBuf> {
    validate_loader_name(&library.loader_name)?;
    let loader_name = library.loader_name.as_os_str();
    let source = &library.source;
    let directory = dependency_guard_base_dir().join(format!(
        "{}-{:016x}",
        sanitize_file_name(loader_name),
        dependency_guard_hash(library)?
    ));
    std::fs::create_dir_all(&directory).map_err(|error| {
        SyncError::new(format!(
            "cannot create runtime dependency guard directory {}: {error}",
            directory.display()
        ))
    })?;
    let target = directory.join(loader_name);
    let temporary = directory.join(format!(
        ".{}.{}.tmp",
        sanitize_file_name(loader_name),
        std::process::id()
    ));
    std::fs::copy(source, &temporary).map_err(|error| {
        SyncError::new(format!(
            "cannot copy runtime dependency {} as {} to {}: {error}",
            source.display(),
            loader_name.to_string_lossy(),
            temporary.display()
        ))
    })?;
    std::fs::rename(&temporary, &target).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        SyncError::new(format!(
            "cannot install runtime dependency guard {}: {error}",
            target.display()
        ))
    })?;
    Ok(directory)
}

fn dependency_guard_base_dir() -> PathBuf {
    std::env::var_os(ENV_DEPENDENCY_GUARD_DIR)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("actrail-tls-runtime-deps"))
}

fn dependency_guard_hash(library: &RuntimeDependencyLibrary) -> SyncResult<u64> {
    let metadata = std::fs::metadata(&library.source).map_err(|error| {
        SyncError::new(format!(
            "cannot stat runtime dependency {}: {error}",
            library.source.display()
        ))
    })?;
    let mut hash = 0xcbf29ce484222325u64;
    hash_bytes(&mut hash, library.loader_name.as_os_str().as_bytes());
    hash_bytes(&mut hash, b"\0");
    hash_bytes(&mut hash, library.source.as_os_str().as_bytes());
    hash_bytes(&mut hash, &metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            hash_bytes(&mut hash, &duration.as_secs().to_le_bytes());
            hash_bytes(&mut hash, &duration.subsec_nanos().to_le_bytes());
        }
    }
    Ok(hash)
}

fn validate_loader_name(loader_name: &OsStr) -> SyncResult<()> {
    let mut components = Path::new(loader_name).components();
    if matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
    {
        Ok(())
    } else {
        Err(SyncError::new(format!(
            "runtime dependency loader name must be a file name: {}",
            loader_name.to_string_lossy()
        )))
    }
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn sanitize_file_name(file_name: &OsStr) -> String {
    file_name
        .as_bytes()
        .iter()
        .map(|byte| {
            let byte = *byte;
            if byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || byte == b'-' {
                char::from(byte)
            } else {
                '_'
            }
        })
        .collect()
}

fn runtime_libraries_need(libraries: &[PathBuf], needed: &str) -> SyncResult<bool> {
    let needle = needed.as_bytes();
    for library in libraries {
        let bytes = std::fs::read(library).map_err(|error| {
            SyncError::new(format!(
                "cannot inspect runtime library {}: {error}",
                library.display()
            ))
        })?;
        if bytes.windows(needle.len()).any(|window| window == needle) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn resolve_system_library(name: &str) -> SyncResult<RuntimeDependencyLibrary> {
    for directory in system_library_dirs() {
        let candidate = directory.join(name);
        if candidate.is_file() {
            let source = std::fs::canonicalize(&candidate).map_err(|error| {
                SyncError::new(format!(
                    "cannot resolve system runtime dependency {}: {error}",
                    candidate.display()
                ))
            })?;
            return Ok(RuntimeDependencyLibrary {
                source,
                loader_name: OsString::from(name),
            });
        }
    }
    Err(SyncError::new(format!(
        "runtime library needs {name}, but no system copy was found; set {ENV_LIBRARY_PATH_PREFIX} or {ENV_SYSTEM_LIBRARY_DIRS}"
    )))
}

fn system_library_dirs() -> Vec<PathBuf> {
    if let Some(value) = std::env::var_os(ENV_SYSTEM_LIBRARY_DIRS) {
        return std::env::split_paths(&value).collect();
    }
    default_system_library_dirs()
        .iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_arch = "x86_64")]
fn default_system_library_dirs() -> &'static [&'static str] {
    &[
        "/lib/x86_64-linux-gnu",
        "/usr/lib/x86_64-linux-gnu",
        "/lib64",
        "/usr/lib64",
        "/lib",
        "/usr/lib",
    ]
}

#[cfg(target_arch = "aarch64")]
fn default_system_library_dirs() -> &'static [&'static str] {
    &[
        "/lib/aarch64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/lib64",
        "/usr/lib64",
        "/lib",
        "/usr/lib",
    ]
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn default_system_library_dirs() -> &'static [&'static str] {
    &["/lib", "/usr/lib"]
}

fn join_paths(paths: &[PathBuf]) -> OsString {
    let mut value = OsString::new();
    for path in paths {
        if !value.is_empty() {
            value.push(":");
        }
        value.push(path);
    }
    value
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
    launch_command_for_binary(command, &plan.binary.path)
}

pub fn launch_command_for_plan_descriptor(
    command: &[OsString],
    plan: &RuntimePlanDescriptor,
) -> SyncResult<Vec<OsString>> {
    launch_command_for_binary(command, &plan.binary)
}

fn launch_command_for_binary(command: &[OsString], binary: &Path) -> SyncResult<Vec<OsString>> {
    let Some(program) = command.first() else {
        return Err(SyncError::new("probe command is empty"));
    };
    let entry = resolve_command_path(program)?;
    let plan_binary = std::fs::canonicalize(binary).map_err(|error| {
        SyncError::new(format!(
            "cannot resolve probe binary {}: {error}",
            binary.display()
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
