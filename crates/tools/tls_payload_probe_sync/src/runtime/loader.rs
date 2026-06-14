//! Dynamic loader interposition for TLS libraries imported after process start.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

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
        if is_openssl_library(&path) {
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

fn is_openssl_library(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("libssl") && name.contains(".so"))
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
    call_loader_with_scan("dlopen", || unsafe { original(filename, flags) })
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
    call_loader_with_scan("dlmopen", || unsafe {
        original(namespace, filename, flags)
    })
}

fn call_loader_with_scan(reason: &'static str, call: impl FnOnce() -> *mut c_void) -> *mut c_void {
    LOADER_GUARD.with(|guard| {
        if guard.get() {
            return call();
        }
        guard.set(true);
        let handle = call();
        if !handle.is_null() {
            after_successful_load(reason);
        }
        guard.set(false);
        handle
    })
}

fn after_successful_load(reason: &str) {
    if config::get().is_none() {
        super::retry_initialize_after_loader_event();
    }
    if config::get().is_some() {
        if let Err(error) = scan_loaded_tls_libraries(reason) {
            output::error_line(&format!(
                "tls_payload_probe_sync dynamic scan failed: {error}\n"
            ));
        }
    }
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
    let address = unsafe { libc::dlsym(libc::RTLD_NEXT, symbol.as_ptr().cast()) } as usize;
    if address == 0 {
        return None;
    }
    cache.store(address, Ordering::Release);
    Some(address)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::is_openssl_library;

    #[test]
    fn openssl_library_match_accepts_soname_paths() {
        assert!(is_openssl_library(Path::new("/usr/lib64/libssl.so.1.1")));
        assert!(is_openssl_library(Path::new(
            "/lib/x86_64-linux-gnu/libssl.so.3"
        )));
    }

    #[test]
    fn openssl_library_match_rejects_non_ssl_libraries() {
        assert!(!is_openssl_library(Path::new(
            "/usr/lib64/libcrypto.so.1.1"
        )));
        assert!(!is_openssl_library(Path::new("/usr/lib64/libssl.a")));
    }
}
