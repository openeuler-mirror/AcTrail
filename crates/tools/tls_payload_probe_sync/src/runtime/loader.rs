//! Dynamic loader interposition for TLS libraries imported after process start.

pub(super) mod exec;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::ffi::CStr;
use std::ffi::c_void;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::runtime::tls::dynamic::binding::resolver;
use crate::runtime::{config, maps, output, ssl};

type DlopenFn = unsafe extern "C" fn(*const libc::c_char, libc::c_int) -> *mut c_void;
type DlmopenFn =
    unsafe extern "C" fn(libc::Lmid_t, *const libc::c_char, libc::c_int) -> *mut c_void;

static DLOPEN_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static DLMOPEN_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SCANNED_LIBRARIES: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();

thread_local! {
    static LOADER_GUARD: Cell<bool> = const { Cell::new(false) };
}

pub(super) unsafe fn open_existing(filename: *const libc::c_char) -> *mut c_void {
    let Some(original) = original_dlopen() else {
        return std::ptr::null_mut();
    };
    unsafe { original(filename, libc::RTLD_LAZY | libc::RTLD_NOLOAD) }
}

pub(super) fn scan_loaded_tls_libraries(reason: &str) -> Result<(), String> {
    for path in maps::executable_mapped_files()? {
        if is_probe_library_candidate(&path) {
            scan_library_once(&path, reason)?;
        }
    }
    Ok(())
}

fn scan_library_once(path: &Path, reason: &str) -> Result<(), String> {
    let path = canonical(path);
    if !claim_library_scan(&path)? {
        return Ok(());
    }
    target_event(format!(
        "sync_dynamic: event=library_scanned reason={reason} path={}\n",
        path.display()
    ));
    match config::runtime_plan_for_binary(&path) {
        Ok(Some(plan)) => {
            target_event(format!(
                "sync_dynamic: event=plan_found provider={} target={} binary={}\n",
                plan.provider,
                plan.target.display(),
                plan.binary.display()
            ));
            if ssl::dynamic_binding_covers_plan(&plan) {
                ssl::register_dynamic_binding_plan(&plan)?;
                target_event(format!(
                    "sync_dynamic: event=dynamic_binding_registered provider={} binary={}\n",
                    plan.provider,
                    plan.binary.display()
                ));
                return Ok(());
            }
            match ssl::install_plan(&plan) {
                Ok(ssl::HookInstallStatus::Installed) => {
                    target_event(format!(
                        "sync_dynamic: event=hook_installed provider={} binary={} points={}\n",
                        plan.provider,
                        plan.binary.display(),
                        plan.points.len()
                    ));
                }
                Ok(ssl::HookInstallStatus::DuplicateSkipped) => {
                    target_event(format!(
                        "sync_dynamic: event=duplicate_plan_skipped provider={} binary={}\n",
                        plan.provider,
                        plan.binary.display()
                    ));
                }
                Err(error) => {
                    target_event(format!(
                        "sync_dynamic: event=hook_install_failed binary={} error={error}\n",
                        plan.binary.display()
                    ));
                    return Err(error);
                }
            }
        }
        Ok(None) => {
            target_event(format!(
                "sync_dynamic: event=plan_unsupported binary={}\n",
                path.display()
            ));
        }
        Err(error) => {
            unclaim_library_scan(&path);
            return Err(error);
        }
    }
    Ok(())
}

fn claim_library_scan(path: &Path) -> Result<bool, String> {
    let mut scanned = SCANNED_LIBRARIES
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .lock()
        .map_err(|_| "dynamic library scan mutex poisoned".to_string())?;
    Ok(scanned.insert(path.to_path_buf()))
}

fn unclaim_library_scan(path: &Path) {
    if let Ok(mut scanned) = SCANNED_LIBRARIES
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .lock()
    {
        scanned.remove(path);
    }
}

fn is_probe_library_candidate(path: &Path) -> bool {
    if is_own_runtime_library(path) {
        return false;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(".so"))
}

fn is_own_runtime_library(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "libactrail_tls_payload_probe_sync.so")
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn target_event(line: String) {
    if config::get().is_some_and(|config| config.should_print_target()) {
        output::event_line(&line);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlopen(filename: *const libc::c_char, flags: libc::c_int) -> *mut c_void {
    let Some(original) = original_dlopen() else {
        output::error_line("tls_payload_probe_sync error: real dlopen not found\n");
        return std::ptr::null_mut();
    };
    call_loader_with_scan("dlopen", filename, || unsafe { original(filename, flags) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlmopen(
    namespace: libc::Lmid_t,
    filename: *const libc::c_char,
    flags: libc::c_int,
) -> *mut c_void {
    let Some(original) = original_dlmopen() else {
        output::error_line("tls_payload_probe_sync error: real dlmopen not found\n");
        return std::ptr::null_mut();
    };
    call_loader_with_scan("dlmopen", filename, || unsafe {
        original(namespace, filename, flags)
    })
}

fn call_loader_with_scan(
    reason: &'static str,
    filename: *const libc::c_char,
    call: impl FnOnce() -> *mut c_void,
) -> *mut c_void {
    LOADER_GUARD.with(|guard| {
        if guard.get() {
            return call();
        }
        guard.set(true);
        prefetch_requested_library_plan(filename);
        let handle = call();
        if !handle.is_null() {
            after_successful_load(reason, filename);
        }
        guard.set(false);
        handle
    })
}

fn prefetch_requested_library_plan(filename: *const libc::c_char) {
    if config::get().is_none() {
        return;
    }
    let Some(path) = requested_library_path(filename) else {
        return;
    };
    if let Err(error) = config::prefetch_runtime_plan_for_binary(&path) {
        output::error_line(&format!(
            "tls_payload_probe_sync dynamic plan prefetch failed: {error}\n"
        ));
    }
}

fn after_successful_load(reason: &str, filename: *const libc::c_char) {
    if config::get().is_none() {
        super::retry_initialize_after_loader_event();
    }
    if config::get().is_some() {
        match scan_requested_library(filename, reason) {
            Ok(true) => {}
            Ok(false) => {}
            Err(error) => {
                output::error_line(&format!(
                    "tls_payload_probe_sync dynamic direct scan failed: {error}\n"
                ));
                return;
            }
        }
        if let Err(error) = scan_loaded_tls_libraries(reason) {
            output::error_line(&format!(
                "tls_payload_probe_sync dynamic scan failed: {error}\n"
            ));
        }
    }
}

fn scan_requested_library(filename: *const libc::c_char, reason: &str) -> Result<bool, String> {
    let Some(path) = requested_library_path(filename) else {
        return Ok(false);
    };
    scan_library_once(&path, reason)?;
    Ok(true)
}

fn requested_library_path(filename: *const libc::c_char) -> Option<PathBuf> {
    if filename.is_null() {
        return None;
    }
    let path = unsafe { CStr::from_ptr(filename) };
    let path = Path::new(std::ffi::OsStr::from_bytes(path.to_bytes()));
    if !path.is_absolute() || !is_probe_library_candidate(path) {
        return None;
    }
    Some(path.to_path_buf())
}

fn original_dlopen() -> Option<DlopenFn> {
    original_symbol(&DLOPEN_ORIGINAL, b"dlopen\0")
        .map(|address| unsafe { std::mem::transmute::<usize, DlopenFn>(address) })
}

fn original_dlmopen() -> Option<DlmopenFn> {
    original_symbol(&DLMOPEN_ORIGINAL, b"dlmopen\0")
        .map(|address| unsafe { std::mem::transmute::<usize, DlmopenFn>(address) })
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::is_probe_library_candidate;

    #[test]
    fn probe_library_match_accepts_shared_object_paths() {
        assert!(is_probe_library_candidate(Path::new(
            "/usr/lib64/libssl.so.1.1"
        )));
        assert!(is_probe_library_candidate(Path::new(
            "/lib/x86_64-linux-gnu/libssl.so.3"
        )));
        assert!(is_probe_library_candidate(Path::new(
            "/tmp/libnetty_tcnative_linux_x86_64.so"
        )));
    }

    #[test]
    fn probe_library_match_rejects_non_shared_object_paths() {
        assert!(!is_probe_library_candidate(Path::new("/usr/bin/java")));
        assert!(!is_probe_library_candidate(Path::new(
            "/usr/lib64/libssl.a"
        )));
        assert!(!is_probe_library_candidate(Path::new(
            "/tmp/libactrail_tls_payload_probe_sync.so"
        )));
    }
}
