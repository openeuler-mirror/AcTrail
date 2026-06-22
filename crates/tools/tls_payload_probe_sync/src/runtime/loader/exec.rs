//! exec-family interposition for preserving TLS capture env.

use std::ffi::{CStr, CString, OsStr, OsString, c_char};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use tls_payload_sync::{ENV_BINARY, ENV_EVENT_SOCKET, ENV_POINTS, ENV_PROVIDER};

use crate::runtime::config;
use crate::runtime::tls::dynamic::binding::resolver;

const JAVA_TOOL_OPTIONS: &str = "JAVA_TOOL_OPTIONS";
const LD_AUDIT: &str = "LD_AUDIT";
const LD_PRELOAD: &str = "LD_PRELOAD";
const PATH_ENV: &str = "PATH";
const TLS_SYNC_PREFIX: &str = "TLS_PAYLOAD_SYNC_";
const ACTRAIL_SYNC_RUNTIME_LIBRARY: &str = "libactrail_tls_payload_probe_sync.so";
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

static EXECVE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static EXECVEAT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static EXECVPE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct EnvEntry {
    key: OsString,
    value: OsString,
}

impl EnvEntry {
    #[cfg(test)]
    pub(in crate::runtime) fn new(key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    #[cfg(test)]
    pub(in crate::runtime) fn key(&self) -> &OsStr {
        &self.key
    }

    fn from_os(key: OsString, value: OsString) -> Self {
        Self { key, value }
    }
}

pub(in crate::runtime) fn merge_exec_env(
    program: impl AsRef<OsStr>,
    child_env: &[EnvEntry],
    current_env: &[EnvEntry],
) -> Vec<EnvEntry> {
    let target_has_capture_plan = target_has_capture_plan(program.as_ref(), child_env, current_env);
    let mut merged = child_env.to_vec();
    merge_tls_sync_env(&mut merged, current_env);
    strip_active_plan_env(&mut merged);
    merge_ld_preload(&mut merged, current_env);
    merge_ld_audit(&mut merged, current_env, target_has_capture_plan);
    if is_java_executable(program.as_ref()) {
        merge_java_tool_options(&mut merged, current_env);
    }
    merged
}

fn merge_tls_sync_env(merged: &mut Vec<EnvEntry>, current_env: &[EnvEntry]) {
    for entry in current_env
        .iter()
        .filter(|entry| os_str_starts_with(&entry.key, TLS_SYNC_PREFIX))
    {
        upsert_env(merged, entry.clone());
    }
}

fn merge_ld_preload(merged: &mut Vec<EnvEntry>, current_env: &[EnvEntry]) {
    let Some(current) = current_env
        .iter()
        .rev()
        .find(|entry| entry.key == LD_PRELOAD)
        .filter(|entry| !entry.value.is_empty())
    else {
        return;
    };
    let Some(position) = merged.iter().position(|entry| entry.key == LD_PRELOAD) else {
        merged.push(current.clone());
        return;
    };
    merged[position].value =
        append_missing_ld_preload_entries(&merged[position].value, &current.value);
}

fn merge_ld_audit(
    merged: &mut Vec<EnvEntry>,
    current_env: &[EnvEntry],
    target_has_capture_plan: bool,
) {
    let mut entries = Vec::new();
    if let Some(existing) = env_value(merged, LD_AUDIT) {
        append_loader_entries(&mut entries, existing, target_has_capture_plan, false);
    }
    if let Some(current) = env_value(current_env, LD_AUDIT) {
        append_loader_entries(&mut entries, current, target_has_capture_plan, false);
    }
    if target_has_capture_plan {
        if let Some(preload) = env_value(current_env, LD_PRELOAD) {
            append_loader_entries(&mut entries, preload, true, true);
        }
    }
    set_loader_env(merged, LD_AUDIT, &entries);
}

fn append_missing_ld_preload_entries(existing: &OsStr, current: &OsStr) -> OsString {
    let existing_entries = split_colon_entries(existing.as_bytes());
    let mut value = existing.as_bytes().to_vec();
    for entry in split_colon_entries(current.as_bytes()) {
        if existing_entries.contains(&entry) {
            continue;
        }
        if !value.is_empty() {
            value.push(b':');
        }
        value.extend_from_slice(entry);
    }
    OsString::from_vec(value)
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
    include_actrail_runtime: bool,
    actrail_runtime_only: bool,
) {
    for entry in split_colon_entries(value.as_bytes()) {
        let actrail_runtime = is_actrail_sync_runtime_entry(entry);
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

fn is_actrail_sync_runtime_entry(entry: &[u8]) -> bool {
    entry
        .rsplit(|byte| *byte == b'/')
        .next()
        .is_some_and(|name| name == ACTRAIL_SYNC_RUNTIME_LIBRARY.as_bytes())
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

fn target_has_capture_plan(
    program: &OsStr,
    child_env: &[EnvEntry],
    current_env: &[EnvEntry],
) -> bool {
    let Some(path) = resolve_exec_target(program, child_env, current_env) else {
        return false;
    };
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
    let Some(env) = merged_envp_for_exec(filename, envp) else {
        return unsafe { original(filename, argv, envp) };
    };
    unsafe { original(filename, argv, env.as_ptr()) }
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
    let Some(env) = merged_envp_for_exec(pathname, envp) else {
        return unsafe { original(dirfd, pathname, argv, envp, flags) };
    };
    unsafe { original(dirfd, pathname, argv, env.as_ptr(), flags) }
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
    let Some(env) = merged_envp_for_exec(file, envp) else {
        return unsafe { original(file, argv, envp) };
    };
    unsafe { original(file, argv, env.as_ptr()) }
}

fn merged_envp_for_exec(
    program: *const libc::c_char,
    child_envp: *const *const libc::c_char,
) -> Option<OwnedEnvp> {
    if program.is_null() {
        return None;
    }
    let program = unsafe { CStr::from_ptr(program) };
    let program = OsStr::from_bytes(program.to_bytes());
    if std::env::var_os(ENV_EVENT_SOCKET).is_none() {
        return None;
    }
    let child = unsafe { envp_entries(child_envp) };
    let current = current_env_entries();
    Some(OwnedEnvp::from_entries(&merge_exec_env(
        program, &child, &current,
    )))
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
