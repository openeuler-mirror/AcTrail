//! Loader interposition for lazy TLS library activation.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static DLOPEN_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static DLMOPEN_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static RETRYING: AtomicBool = AtomicBool::new(false);

type DlopenFn = unsafe extern "C" fn(*const libc::c_char, libc::c_int) -> *mut c_void;
type DlmopenFn =
    unsafe extern "C" fn(libc::Lmid_t, *const libc::c_char, libc::c_int) -> *mut c_void;

pub(super) unsafe fn open_existing(filename: *const libc::c_char) -> *mut c_void {
    let Some(original) = original_dlopen() else {
        return std::ptr::null_mut();
    };
    unsafe { original(filename, libc::RTLD_LAZY | libc::RTLD_NOLOAD) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlopen(filename: *const libc::c_char, flags: libc::c_int) -> *mut c_void {
    let handle = match original_dlopen() {
        Some(original) => unsafe { original(filename, flags) },
        None => std::ptr::null_mut(),
    };
    retry_after_successful_load(handle);
    handle
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlmopen(
    namespace: libc::Lmid_t,
    filename: *const libc::c_char,
    flags: libc::c_int,
) -> *mut c_void {
    let handle = match original_dlmopen() {
        Some(original) => unsafe { original(namespace, filename, flags) },
        None => std::ptr::null_mut(),
    };
    retry_after_successful_load(handle);
    handle
}

fn retry_after_successful_load(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    if RETRYING.swap(true, Ordering::AcqRel) {
        return;
    }
    super::retry_initialize_after_loader_event();
    RETRYING.store(false, Ordering::Release);
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
