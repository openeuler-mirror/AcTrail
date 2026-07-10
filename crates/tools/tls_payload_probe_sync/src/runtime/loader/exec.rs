//! exec-family interposition for preserving TLS capture env.

use std::ffi::{CStr, CString, OsStr, OsString, c_char};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use tls_payload_sync::{
    ENV_BINARY, ENV_EVENT_SOCKET, ENV_LIBRARY_PATH_PREFIX, ENV_LIBRARY_PATH_PREFIX_GLIBC,
    ENV_LIBRARY_PATH_PREFIX_MUSL, ENV_POINTS, ENV_PROVIDER, ENV_RUNTIME_GLIBC_LIBRARY,
    ENV_RUNTIME_MUSL_LIBRARY, LibcFamily, RUNTIME_GLIBC_LIBRARY_NAME, RUNTIME_MUSL_LIBRARY_NAME,
};

use crate::runtime::tls::dynamic::binding::resolver;
use crate::runtime::{config, output};

const JAVA_TOOL_OPTIONS: &str = "JAVA_TOOL_OPTIONS";
const LD_AUDIT: &str = "LD_AUDIT";
const LD_LIBRARY_PATH: &str = "LD_LIBRARY_PATH";
const LD_PRELOAD: &str = "LD_PRELOAD";
const PATH_ENV: &str = "PATH";
const TLS_SYNC_PREFIX: &str = "TLS_PAYLOAD_SYNC_";
const ACTRAIL_AGENT_JAR_MARKER: &str = "actrail-java-payload-agent-";

type ExecveFn = unsafe extern "C" fn(
    *const libc::c_char,
    *const *const libc::c_char,
    *const *const libc::c_char,
) -> libc::c_int;
type ExecveatFn = unsafe extern "C" fn(
    libc::c_int,
    *const libc::c_char,
    *const *const libc::c_char,
    *const *const libc::c_char,
    libc::c_int,
) -> libc::c_int;
type ExecvpeFn = unsafe extern "C" fn(
    *const libc::c_char,
    *const *const libc::c_char,
    *const *const libc::c_char,
) -> libc::c_int;
type PosixSpawnFn = unsafe extern "C" fn(
    *mut libc::pid_t,
    *const libc::c_char,
    *const libc::posix_spawn_file_actions_t,
    *const libc::posix_spawnattr_t,
    *const *const libc::c_char,
    *const *const libc::c_char,
) -> libc::c_int;

static EXECVE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static EXECVEAT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static EXECVPE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static POSIX_SPAWN_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static POSIX_SPAWNP_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    static mut environ: *mut *mut libc::c_char;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct EnvEntry {
    key: OsString,
    value: OsString,
}

impl EnvEntry {
    fn from_os(key: OsString, value: OsString) -> Self {
        Self { key, value }
    }
}

fn merge_exec_env_checked(
    program: &OsStr,
    child_env: &[EnvEntry],
    current_env: &[EnvEntry],
) -> Result<Vec<EnvEntry>, String> {
    merge_exec_env_inner(program, child_env, current_env, true)
}

fn merge_exec_env_inner(
    program: &OsStr,
    child_env: &[EnvEntry],
    current_env: &[EnvEntry],
    fail_on_unknown_runtime: bool,
) -> Result<Vec<EnvEntry>, String> {
    let target = resolve_exec_target(program, child_env, current_env);
    let target_has_capture_plan = target_has_capture_plan(target.as_deref());
    let runtime_family = target_runtime_family(
        program,
        target.as_deref(),
        child_env,
        current_env,
        fail_on_unknown_runtime,
    )?;
    let mut merged = child_env.to_vec();
    merge_tls_sync_env(&mut merged, current_env);
    strip_active_plan_env(&mut merged);
    merge_ld_library_path_prefix(&mut merged, current_env, runtime_family);
    merge_ld_preload(&mut merged, current_env, runtime_family)?;
    merge_ld_audit(
        &mut merged,
        current_env,
        runtime_family,
        target_has_capture_plan,
    )?;
    if is_java_executable(program) {
        merge_java_tool_options(&mut merged, current_env);
    }
    Ok(merged)
}

fn merge_tls_sync_env(merged: &mut Vec<EnvEntry>, current_env: &[EnvEntry]) {
    for entry in current_env
        .iter()
        .filter(|entry| os_str_starts_with(&entry.key, TLS_SYNC_PREFIX))
    {
        upsert_env(merged, entry.clone());
    }
}

fn merge_ld_library_path_prefix(
    merged: &mut Vec<EnvEntry>,
    current_env: &[EnvEntry],
    family: LibcFamily,
) {
    let existing_entries = env_value(merged, LD_LIBRARY_PATH)
        .map(|existing| {
            split_colon_entries(existing.as_bytes())
                .into_iter()
                .map(Vec::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut entries = match family {
        LibcFamily::Glibc => {
            let Some(prefix) = glibc_library_path_prefix(current_env) else {
                return;
            };
            let mut entries = split_colon_entries(prefix.as_bytes())
                .into_iter()
                .map(Vec::from)
                .collect::<Vec<_>>();
            append_loader_entry_bytes(&mut entries, existing_entries);
            entries
        }
        LibcFamily::Musl => {
            let glibc_prefix = glibc_library_path_prefix(current_env)
                .map(|prefix| {
                    split_colon_entries(prefix.as_bytes())
                        .into_iter()
                        .map(Vec::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut entries = existing_entries
                .into_iter()
                .filter(|entry| !glibc_prefix.iter().any(|prefix| prefix == entry))
                .collect::<Vec<_>>();
            if let Some(prefix) = env_value(current_env, ENV_LIBRARY_PATH_PREFIX_MUSL) {
                let mut prefixed = split_colon_entries(prefix.as_bytes())
                    .into_iter()
                    .map(Vec::from)
                    .collect::<Vec<_>>();
                append_loader_entry_bytes(&mut prefixed, entries);
                entries = prefixed;
            }
            entries
        }
    };
    dedup_loader_entries(&mut entries);
    set_loader_env(merged, LD_LIBRARY_PATH, &entries);
}

fn glibc_library_path_prefix<'a>(current_env: &'a [EnvEntry]) -> Option<&'a OsStr> {
    env_value(current_env, ENV_LIBRARY_PATH_PREFIX_GLIBC)
        .or_else(|| env_value(current_env, ENV_LIBRARY_PATH_PREFIX))
}

fn merge_ld_preload(
    merged: &mut Vec<EnvEntry>,
    current_env: &[EnvEntry],
    family: LibcFamily,
) -> Result<(), String> {
    let selected_runtime = selected_runtime_library(current_env, family)?;
    let mut entries = env_value(merged, LD_PRELOAD)
        .map(|existing| loader_entries_without_actrail_runtime(existing, current_env))
        .unwrap_or_default();
    if let Some(current) = env_value(current_env, LD_PRELOAD) {
        append_loader_entry_bytes(
            &mut entries,
            loader_entries_without_actrail_runtime(current, current_env),
        );
    }
    append_loader_entry_bytes(&mut entries, vec![selected_runtime.as_bytes().to_vec()]);
    dedup_loader_entries(&mut entries);
    set_loader_env(merged, LD_PRELOAD, &entries);
    Ok(())
}

fn selected_runtime_library(
    current_env: &[EnvEntry],
    family: LibcFamily,
) -> Result<OsString, String> {
    let key = match family {
        LibcFamily::Glibc => ENV_RUNTIME_GLIBC_LIBRARY,
        LibcFamily::Musl => ENV_RUNTIME_MUSL_LIBRARY,
    };
    if let Some(path) = env_value(current_env, key) {
        let path = path.to_os_string();
        if Path::new(&path).is_file() {
            return Ok(path);
        }
        return Err(format!(
            "{key} is not visible in target namespace: {}",
            path.to_string_lossy()
        ));
    }
    if family == LibcFamily::Glibc {
        if let Some(path) = current_actrail_runtime(current_env) {
            return Ok(path);
        }
    }
    Err(format!(
        "{} TLS sync runtime library is not configured; set {key}",
        family.as_str()
    ))
}

fn current_actrail_runtime(current_env: &[EnvEntry]) -> Option<OsString> {
    env_value(current_env, LD_PRELOAD)?
        .as_bytes()
        .split(|byte| *byte == b':')
        .find(|entry| is_actrail_sync_runtime_entry(entry, current_env))
        .map(|entry| OsString::from_vec(entry.to_vec()))
}

fn loader_entries_without_actrail_runtime(value: &OsStr, current_env: &[EnvEntry]) -> Vec<Vec<u8>> {
    split_colon_entries(value.as_bytes())
        .into_iter()
        .filter(|entry| !is_actrail_sync_runtime_entry(entry, current_env))
        .map(Vec::from)
        .collect()
}

fn append_loader_entry_bytes(entries: &mut Vec<Vec<u8>>, candidates: Vec<Vec<u8>>) {
    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }
        if entries.iter().any(|existing| existing == &candidate) {
            continue;
        }
        entries.push(candidate);
    }
}

fn dedup_loader_entries(entries: &mut Vec<Vec<u8>>) {
    let mut deduped = Vec::new();
    for entry in std::mem::take(entries) {
        if deduped.iter().any(|existing| existing == &entry) {
            continue;
        }
        deduped.push(entry);
    }
    *entries = deduped;
}

fn merge_ld_audit(
    merged: &mut Vec<EnvEntry>,
    current_env: &[EnvEntry],
    family: LibcFamily,
    target_has_capture_plan: bool,
) -> Result<(), String> {
    if family == LibcFamily::Musl {
        set_loader_env(merged, LD_AUDIT, &[]);
        return Ok(());
    }
    let mut entries = Vec::new();
    if let Some(existing) = env_value(merged, LD_AUDIT) {
        append_loader_entries(
            &mut entries,
            existing,
            current_env,
            target_has_capture_plan,
            false,
        );
    }
    if let Some(current) = env_value(current_env, LD_AUDIT) {
        append_loader_entries(
            &mut entries,
            current,
            current_env,
            target_has_capture_plan,
            false,
        );
    }
    if target_has_capture_plan {
        let selected_runtime = selected_runtime_library(current_env, family)?;
        append_loader_entry_bytes(&mut entries, vec![selected_runtime.as_bytes().to_vec()]);
    }
    set_loader_env(merged, LD_AUDIT, &entries);
    Ok(())
}

fn target_runtime_family(
    program: &OsStr,
    target: Option<&Path>,
    child_env: &[EnvEntry],
    current_env: &[EnvEntry],
    fail_on_unknown_runtime: bool,
) -> Result<LibcFamily, String> {
    let path_value = env_value(child_env, PATH_ENV).or_else(|| env_value(current_env, PATH_ENV));
    let Some(target) = target else {
        if fail_on_unknown_runtime {
            return Err(format!(
                "cannot resolve exec target {} for TLS sync runtime selection",
                program.to_string_lossy()
            ));
        }
        return Ok(LibcFamily::Glibc);
    };
    match tls_payload_sync::target_runtime_for_path(target, path_value) {
        Ok(runtime) => Ok(runtime.libc),
        Err(error) if fail_on_unknown_runtime => Err(error.to_string()),
        Err(_) => Ok(LibcFamily::Glibc),
    }
}

fn split_colon_entries(value: &[u8]) -> Vec<&[u8]> {
    value
        .split(|byte| *byte == b':')
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn append_loader_entries(
    entries: &mut Vec<Vec<u8>>,
    value: &OsStr,
    current_env: &[EnvEntry],
    include_actrail_runtime: bool,
    actrail_runtime_only: bool,
) {
    for entry in split_colon_entries(value.as_bytes()) {
        let actrail_runtime = is_actrail_sync_runtime_entry(entry, current_env);
        if actrail_runtime_only && !actrail_runtime {
            continue;
        }
        if !include_actrail_runtime && actrail_runtime {
            continue;
        }
        if entries.iter().any(|existing| existing.as_slice() == entry) {
            continue;
        }
        entries.push(entry.to_vec());
    }
}

fn set_loader_env(merged: &mut Vec<EnvEntry>, key: &str, entries: &[Vec<u8>]) {
    let position = merged.iter().position(|entry| entry.key == key);
    if entries.is_empty() {
        if let Some(position) = position {
            merged.remove(position);
        }
        return;
    }
    let value = join_loader_entries(entries);
    if let Some(position) = position {
        merged[position].value = value;
    } else {
        merged.push(EnvEntry::from_os(OsString::from(key), value));
    }
}

fn join_loader_entries(entries: &[Vec<u8>]) -> OsString {
    let mut value = Vec::new();
    for entry in entries {
        if !value.is_empty() {
            value.push(b':');
        }
        value.extend_from_slice(entry);
    }
    OsString::from_vec(value)
}

fn is_actrail_sync_runtime_entry(entry: &[u8], current_env: &[EnvEntry]) -> bool {
    if env_value(current_env, ENV_RUNTIME_GLIBC_LIBRARY)
        .is_some_and(|path| path.as_bytes() == entry)
        || env_value(current_env, ENV_RUNTIME_MUSL_LIBRARY)
            .is_some_and(|path| path.as_bytes() == entry)
    {
        return true;
    }
    entry
        .rsplit(|byte| *byte == b'/')
        .next()
        .is_some_and(|name| {
            name == RUNTIME_GLIBC_LIBRARY_NAME.as_bytes()
                || name == RUNTIME_MUSL_LIBRARY_NAME.as_bytes()
        })
}

fn merge_java_tool_options(merged: &mut Vec<EnvEntry>, current_env: &[EnvEntry]) {
    let Some(agent_options) = current_env
        .iter()
        .rev()
        .find(|entry| entry.key == JAVA_TOOL_OPTIONS)
        .map(|entry| actrail_java_agent_options(&entry.value))
        .filter(|options| !options.is_empty())
    else {
        return;
    };
    let Some(position) = merged
        .iter()
        .position(|entry| entry.key == JAVA_TOOL_OPTIONS)
    else {
        merged.push(EnvEntry::from_os(
            OsString::from(JAVA_TOOL_OPTIONS),
            OsString::from(agent_options.join(" ")),
        ));
        return;
    };
    let value = append_missing_java_agent_options(&merged[position].value, &agent_options);
    merged[position].value = value;
}

fn append_missing_java_agent_options(existing: &OsStr, agent_options: &[String]) -> OsString {
    let Some(existing_text) = existing.to_str() else {
        return existing.to_os_string();
    };
    let mut value = existing_text.to_string();
    for option in agent_options {
        if existing_text
            .split_whitespace()
            .any(|token| token == option)
        {
            continue;
        }
        if !value.is_empty() {
            value.push(' ');
        }
        value.push_str(option);
    }
    OsString::from(value)
}

fn actrail_java_agent_options(value: &OsStr) -> Vec<String> {
    value
        .to_str()
        .map(|value| {
            value
                .split_whitespace()
                .filter(|token| {
                    token.starts_with("-javaagent:") && token.contains(ACTRAIL_AGENT_JAR_MARKER)
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn target_has_capture_plan(path: Option<&Path>) -> bool {
    let Some(path) = path else { return false };
    matches!(config::runtime_plan_for_binary(&path), Ok(Some(_)))
}

fn resolve_exec_target(
    program: &OsStr,
    child_env: &[EnvEntry],
    current_env: &[EnvEntry],
) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let path = Path::new(program);
    if program.as_bytes().contains(&b'/') {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        return Some(canonical(candidate));
    }
    if let Some(candidate) = current_dir_candidate(path) {
        return Some(candidate);
    }
    let path_value = env_value(child_env, PATH_ENV).or_else(|| env_value(current_env, PATH_ENV))?;
    for directory in std::env::split_paths(path_value) {
        let candidate = directory.join(path);
        if candidate.is_file() {
            return Some(canonical(candidate));
        }
    }
    None
}

fn current_dir_candidate(path: &Path) -> Option<PathBuf> {
    let candidate = std::env::current_dir().ok()?.join(path);
    candidate.is_file().then(|| canonical(candidate))
}

fn canonical(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn strip_active_plan_env(env: &mut Vec<EnvEntry>) {
    env.retain(|entry| {
        entry.key != OsStr::new(ENV_BINARY)
            && entry.key != OsStr::new(ENV_PROVIDER)
            && entry.key != OsStr::new(ENV_POINTS)
    });
}

fn env_value<'a>(env: &'a [EnvEntry], key: &str) -> Option<&'a OsStr> {
    env.iter()
        .rev()
        .find(|entry| entry.key == key)
        .map(|entry| entry.value.as_os_str())
        .filter(|value| !value.is_empty())
}

fn upsert_env(env: &mut Vec<EnvEntry>, entry: EnvEntry) {
    if let Some(existing) = env.iter_mut().find(|candidate| candidate.key == entry.key) {
        existing.value = entry.value;
    } else {
        env.push(entry);
    }
}

fn is_java_executable(program: &OsStr) -> bool {
    PathLike::new(program).file_name().is_some_and(|file_name| {
        file_name == OsStr::new("java") || file_name == OsStr::new("java.exe")
    })
}

fn os_str_starts_with(value: &OsStr, prefix: &str) -> bool {
    value
        .to_str()
        .is_some_and(|value| value.starts_with(prefix))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execve(
    filename: *const libc::c_char,
    argv: *const *const libc::c_char,
    envp: *const *const libc::c_char,
) -> libc::c_int {
    let Some(original) = original_execve() else {
        return missing_original();
    };
    match merged_envp_for_exec(filename, envp) {
        Ok(Some(env)) => unsafe { original(filename, argv, env.as_ptr()) },
        Ok(None) => unsafe { original(filename, argv, envp) },
        Err(error) => exec_env_error(error),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execveat(
    dirfd: libc::c_int,
    pathname: *const libc::c_char,
    argv: *const *const libc::c_char,
    envp: *const *const libc::c_char,
    flags: libc::c_int,
) -> libc::c_int {
    let Some(original) = original_execveat() else {
        return missing_original();
    };
    match merged_envp_for_execveat(dirfd, pathname, flags, envp) {
        Ok(Some(env)) => unsafe { original(dirfd, pathname, argv, env.as_ptr(), flags) },
        Ok(None) => unsafe { original(dirfd, pathname, argv, envp, flags) },
        Err(error) => exec_env_error(error),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execvpe(
    file: *const libc::c_char,
    argv: *const *const libc::c_char,
    envp: *const *const libc::c_char,
) -> libc::c_int {
    let Some(original) = original_execvpe() else {
        return missing_original();
    };
    match merged_envp_for_exec(file, envp) {
        Ok(Some(env)) => unsafe { original(file, argv, env.as_ptr()) },
        Ok(None) => unsafe { original(file, argv, envp) },
        Err(error) => exec_env_error(error),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execv(
    path: *const libc::c_char,
    argv: *const *const libc::c_char,
) -> libc::c_int {
    let Some(original) = original_execve() else {
        return missing_original();
    };
    let envp = current_environ();
    match merged_envp_for_exec(path, envp) {
        Ok(Some(env)) => unsafe { original(path, argv, env.as_ptr()) },
        Ok(None) => unsafe { original(path, argv, envp) },
        Err(error) => exec_env_error(error),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execvp(
    file: *const libc::c_char,
    argv: *const *const libc::c_char,
) -> libc::c_int {
    let Some(original) = original_execvpe() else {
        return missing_original();
    };
    let envp = current_environ();
    match merged_envp_for_exec(file, envp) {
        Ok(Some(env)) => unsafe { original(file, argv, env.as_ptr()) },
        Ok(None) => unsafe { original(file, argv, envp) },
        Err(error) => exec_env_error(error),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn(
    pid: *mut libc::pid_t,
    path: *const libc::c_char,
    file_actions: *const libc::posix_spawn_file_actions_t,
    attrp: *const libc::posix_spawnattr_t,
    argv: *const *const libc::c_char,
    envp: *const *const libc::c_char,
) -> libc::c_int {
    let Some(original) = original_posix_spawn() else {
        return libc::ENOSYS;
    };
    match merged_envp_for_exec(path, envp) {
        Ok(Some(env)) => unsafe { original(pid, path, file_actions, attrp, argv, env.as_ptr()) },
        Ok(None) => unsafe { original(pid, path, file_actions, attrp, argv, envp) },
        Err(error) => posix_spawn_env_error(error),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnp(
    pid: *mut libc::pid_t,
    file: *const libc::c_char,
    file_actions: *const libc::posix_spawn_file_actions_t,
    attrp: *const libc::posix_spawnattr_t,
    argv: *const *const libc::c_char,
    envp: *const *const libc::c_char,
) -> libc::c_int {
    let Some(original) = original_posix_spawnp() else {
        return libc::ENOSYS;
    };
    match merged_envp_for_exec(file, envp) {
        Ok(Some(env)) => unsafe { original(pid, file, file_actions, attrp, argv, env.as_ptr()) },
        Ok(None) => unsafe { original(pid, file, file_actions, attrp, argv, envp) },
        Err(error) => posix_spawn_env_error(error),
    }
}

fn merged_envp_for_exec(
    program: *const libc::c_char,
    child_envp: *const *const libc::c_char,
) -> Result<Option<OwnedEnvp>, String> {
    if program.is_null() {
        return Ok(None);
    }
    let program = unsafe { CStr::from_ptr(program) };
    let program = OsStr::from_bytes(program.to_bytes());
    if std::env::var_os(ENV_EVENT_SOCKET).is_none() {
        return Ok(None);
    }
    let child = unsafe { envp_entries(child_envp) };
    let current = current_env_entries();
    merge_exec_env_checked(program, &child, &current).map(|env| Some(OwnedEnvp::from_entries(&env)))
}

fn merged_envp_for_execveat(
    dirfd: libc::c_int,
    pathname: *const libc::c_char,
    flags: libc::c_int,
    child_envp: *const *const libc::c_char,
) -> Result<Option<OwnedEnvp>, String> {
    if pathname.is_null() {
        return Ok(None);
    }
    let pathname = unsafe { CStr::from_ptr(pathname) };
    let program = execveat_detection_program(dirfd, pathname, flags);
    if std::env::var_os(ENV_EVENT_SOCKET).is_none() {
        return Ok(None);
    }
    let child = unsafe { envp_entries(child_envp) };
    let current = current_env_entries();
    merge_exec_env_checked(program.as_os_str(), &child, &current)
        .map(|env| Some(OwnedEnvp::from_entries(&env)))
}

fn execveat_detection_program(dirfd: libc::c_int, pathname: &CStr, flags: libc::c_int) -> OsString {
    let path = OsStr::from_bytes(pathname.to_bytes());
    if !path.is_empty() && (path.as_bytes()[0] == b'/' || dirfd == libc::AT_FDCWD) {
        return path.to_os_string();
    }
    if path.is_empty() && (flags & libc::AT_EMPTY_PATH) == 0 {
        return path.to_os_string();
    }
    let mut resolved = OsString::from(format!("/proc/self/fd/{dirfd}"));
    if !path.is_empty() {
        resolved.push("/");
        resolved.push(path);
    }
    resolved
}

fn exec_env_error(error: String) -> libc::c_int {
    output::error_line(&format!("tls_payload_probe_sync exec env error: {error}\n"));
    set_errno(libc::ENOENT);
    -1
}

fn posix_spawn_env_error(error: String) -> libc::c_int {
    output::error_line(&format!(
        "tls_payload_probe_sync posix_spawn env error: {error}\n"
    ));
    libc::ENOENT
}

fn set_errno(value: libc::c_int) {
    unsafe {
        *libc::__errno_location() = value;
    }
}

fn current_environ() -> *const *const libc::c_char {
    unsafe { environ.cast_const().cast() }
}

unsafe fn envp_entries(envp: *const *const libc::c_char) -> Vec<EnvEntry> {
    if envp.is_null() {
        return Vec::new();
    }
    let mut entries = Vec::new();
    let mut cursor = envp;
    while unsafe { !(*cursor).is_null() } {
        let raw = unsafe { CStr::from_ptr(*cursor) }.to_bytes();
        if let Some((key, value)) = split_env(raw) {
            entries.push(EnvEntry::from_os(
                OsString::from_vec(key.to_vec()),
                OsString::from_vec(value.to_vec()),
            ));
        }
        cursor = unsafe { cursor.add(1) };
    }
    entries
}

fn current_env_entries() -> Vec<EnvEntry> {
    std::env::vars_os()
        .map(|(key, value)| EnvEntry::from_os(key, value))
        .collect()
}

fn split_env(raw: &[u8]) -> Option<(&[u8], &[u8])> {
    let position = raw.iter().position(|byte| *byte == b'=')?;
    Some((&raw[..position], &raw[position + 1..]))
}

struct OwnedEnvp {
    _entries: Vec<CString>,
    pointers: Vec<*const c_char>,
}

impl OwnedEnvp {
    fn from_entries(entries: &[EnvEntry]) -> Self {
        let entries = entries
            .iter()
            .map(|entry| {
                let mut raw = entry.key.as_os_str().as_bytes().to_vec();
                raw.push(b'=');
                raw.extend_from_slice(entry.value.as_os_str().as_bytes());
                CString::new(raw).expect("env entries from C strings do not contain interior NUL")
            })
            .collect::<Vec<_>>();
        let pointers = entries
            .iter()
            .map(|entry| entry.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        Self {
            _entries: entries,
            pointers,
        }
    }

    fn as_ptr(&self) -> *const *const c_char {
        self.pointers.as_ptr()
    }
}

struct PathLike<'a> {
    value: &'a OsStr,
}

impl<'a> PathLike<'a> {
    fn new(value: &'a OsStr) -> Self {
        Self { value }
    }

    fn file_name(&self) -> Option<&'a OsStr> {
        let bytes = self.value.as_bytes();
        let start = bytes
            .iter()
            .rposition(|byte| *byte == b'/')
            .map(|position| position + 1)
            .unwrap_or(0);
        if start >= bytes.len() {
            return None;
        }
        Some(OsStr::from_bytes(&bytes[start..]))
    }
}

fn original_execve() -> Option<ExecveFn> {
    original_symbol(&EXECVE_ORIGINAL, b"execve\0")
        .map(|address| unsafe { std::mem::transmute::<usize, ExecveFn>(address) })
}

fn original_execveat() -> Option<ExecveatFn> {
    original_symbol(&EXECVEAT_ORIGINAL, b"execveat\0")
        .map(|address| unsafe { std::mem::transmute::<usize, ExecveatFn>(address) })
}

fn original_execvpe() -> Option<ExecvpeFn> {
    original_symbol(&EXECVPE_ORIGINAL, b"execvpe\0")
        .map(|address| unsafe { std::mem::transmute::<usize, ExecvpeFn>(address) })
}

fn original_posix_spawn() -> Option<PosixSpawnFn> {
    original_symbol(&POSIX_SPAWN_ORIGINAL, b"posix_spawn\0")
        .map(|address| unsafe { std::mem::transmute::<usize, PosixSpawnFn>(address) })
}

fn original_posix_spawnp() -> Option<PosixSpawnFn> {
    original_symbol(&POSIX_SPAWNP_ORIGINAL, b"posix_spawnp\0")
        .map(|address| unsafe { std::mem::transmute::<usize, PosixSpawnFn>(address) })
}

fn original_symbol(cache: &AtomicUsize, symbol: &[u8]) -> Option<usize> {
    let cached = cache.load(Ordering::Acquire);
    if cached != 0 {
        return Some(cached);
    }
    let name = symbol
        .strip_suffix(b"\0")
        .and_then(|symbol| std::str::from_utf8(symbol).ok())?;
    let address = resolver::libc_symbol(name)?;
    if address == 0 {
        return None;
    }
    cache.store(address, Ordering::Release);
    Some(address)
}

fn missing_original() -> libc::c_int {
    -1
}
