use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::runtime::tls::dynamic::binding::resolver;
use crate::runtime::tls::dynamic::core::capture::{
    SslReadExFn, SslReadFn, SslWriteExFn, SslWriteFn, abort_runtime,
};
use crate::runtime::{loader, maps};

use super::configured_openssl_binary;

static SSL_WRITE_CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_EX_CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_EX_CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_NEXT: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_EX_NEXT: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_NEXT: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_EX_NEXT: AtomicUsize = AtomicUsize::new(0);

pub(super) unsafe fn interposed_ssl_write(capture: bool) -> SslWriteFn {
    let address = interposed_symbol(
        capture,
        &SSL_WRITE_CONFIGURED,
        &SSL_WRITE_NEXT,
        b"SSL_write\0",
    );
    unsafe { std::mem::transmute(address) }
}

pub(super) unsafe fn interposed_ssl_write_ex(capture: bool) -> SslWriteExFn {
    let address = interposed_symbol(
        capture,
        &SSL_WRITE_EX_CONFIGURED,
        &SSL_WRITE_EX_NEXT,
        b"SSL_write_ex\0",
    );
    unsafe { std::mem::transmute(address) }
}

pub(super) unsafe fn interposed_ssl_read(capture: bool) -> SslReadFn {
    let address = interposed_symbol(capture, &SSL_READ_CONFIGURED, &SSL_READ_NEXT, b"SSL_read\0");
    unsafe { std::mem::transmute(address) }
}

pub(super) unsafe fn interposed_ssl_read_ex(capture: bool) -> SslReadExFn {
    let address = interposed_symbol(
        capture,
        &SSL_READ_EX_CONFIGURED,
        &SSL_READ_EX_NEXT,
        b"SSL_read_ex\0",
    );
    unsafe { std::mem::transmute(address) }
}

fn interposed_symbol(
    capture: bool,
    configured_cache: &AtomicUsize,
    next_cache: &AtomicUsize,
    symbol: &'static [u8],
) -> usize {
    if capture {
        configured_symbol(configured_cache, symbol)
    } else {
        next_symbol(next_cache, symbol)
    }
}

fn configured_symbol(cache: &AtomicUsize, symbol: &'static [u8]) -> usize {
    let cached = cache.load(Ordering::Acquire);
    if cached != 0 {
        return cached;
    }
    let Some(binary_path) = configured_openssl_binary() else {
        abort_runtime("dynamic OpenSSL capture is active without a configured OpenSSL binary");
    };
    let binary = CString::new(binary_path.as_os_str().as_bytes()).unwrap_or_else(|_| {
        abort_runtime(&format!(
            "configured OpenSSL binary path contains an interior NUL: {}",
            binary_path.display()
        ))
    });
    let handle = unsafe { loader::open_existing(binary.as_ptr()) };
    if handle.is_null() {
        abort_runtime(&format!(
            "configured OpenSSL binary is not loaded: {}",
            binary_path.display()
        ));
    }
    let address = unsafe { resolver::real_dlsym(handle, symbol.as_ptr().cast()) } as usize;
    if address == 0 {
        abort_runtime(&format!(
            "configured OpenSSL binary {} does not export {}",
            binary_path.display(),
            symbol_name(symbol)
        ));
    }
    cache.store(address, Ordering::Release);
    address
}

fn next_symbol(cache: &AtomicUsize, symbol: &'static [u8]) -> usize {
    let cached = cache.load(Ordering::Acquire);
    if cached != 0 {
        return cached;
    }
    let address = unsafe { resolver::real_dlsym(libc::RTLD_NEXT, symbol.as_ptr().cast()) } as usize;
    if address == 0 {
        if let Some(address) = loaded_tls_symbol(symbol) {
            cache.store(address, Ordering::Release);
            return address;
        }
        abort_runtime(&format!(
            "dynamic OpenSSL pass-through cannot resolve {}",
            symbol_name(symbol)
        ));
    }
    cache.store(address, Ordering::Release);
    address
}

fn loaded_tls_symbol(symbol: &'static [u8]) -> Option<usize> {
    for path in maps::executable_mapped_files().ok()? {
        if !is_tls_library_candidate(&path) {
            continue;
        }
        let path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let handle = unsafe { loader::open_existing(path.as_ptr()) };
        if handle.is_null() {
            continue;
        }
        let address = unsafe { resolver::real_dlsym(handle, symbol.as_ptr().cast()) } as usize;
        if address != 0 {
            return Some(address);
        }
    }
    None
}

fn is_tls_library_candidate(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(".so") && name != "libactrail_tls_payload_probe_sync.so")
}

fn symbol_name(symbol: &'static [u8]) -> &'static str {
    let raw = symbol.strip_suffix(b"\0").unwrap_or(symbol);
    std::str::from_utf8(raw).unwrap_or("<invalid>")
}
