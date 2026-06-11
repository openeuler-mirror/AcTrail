//! OpenSSL/BoringSSL ABI hook handlers.

use std::ffi::{CString, c_void};
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicUsize, Ordering};

use tls_payload_core::PayloadDirection;

use crate::runtime::config::{self, HookPoint};
use crate::runtime::decision::{RuntimeAction, decide_payload};
use crate::runtime::{hook, maps, output, rustls};

static SSL_WRITE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_EX_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_EX_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_EX_CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_EX_CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_NEXT: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_EX_NEXT: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_NEXT: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_EX_NEXT: AtomicUsize = AtomicUsize::new(0);

type SslWriteFn = unsafe extern "C" fn(*mut c_void, *const c_void, libc::c_int) -> libc::c_int;
type SslWriteExFn =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut usize) -> libc::c_int;
type SslReadFn = unsafe extern "C" fn(*mut c_void, *mut c_void, libc::c_int) -> libc::c_int;
type SslReadExFn = unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut usize) -> libc::c_int;

pub(super) fn install_hooks(points: &[HookPoint]) -> Result<(), String> {
    let config = config::get().ok_or_else(|| "runtime config is not initialized".to_string())?;
    if config.provider() == "openssl" && !config.inline_hooks() {
        return Ok(());
    }
    let skip_ssl_read = config.provider() == "openssl"
        && points
            .iter()
            .any(|point| point.symbol.as_str() == "SSL_read_ex");
    for point in points {
        if skip_ssl_read && point.symbol == "SSL_read" {
            continue;
        }
        let mut address = maps::runtime_address(config.binary(), point.file_offset)?;
        if config.provider() == "openssl" && point.symbol == "SSL_read_ex" {
            address = openssl_ssl_read_ex_impl(address);
        }
        let trampoline = if rustls::can_handle(&point.symbol) {
            rustls::install(&point.symbol, address)?
        } else {
            let replacement = replacement_for_symbol(&point.symbol)?;
            let trampoline = hook::install(address, replacement)?;
            set_original(&point.symbol, trampoline)?;
            trampoline
        };
        if config.should_print_target() {
            output::event_line(&format!(
                "sync_hook: symbol={} direction={} address=0x{address:x} trampoline=0x{trampoline:x}\n",
                point.symbol,
                point.direction.as_str(),
            ));
        }
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_write(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: libc::c_int,
) -> libc::c_int {
    let capture = dynamic_openssl_capture_enabled();
    let original = unsafe { interposed_ssl_write(capture) };
    if capture {
        unsafe { ssl_write_with(original, ssl, buffer, length) }
    } else {
        unsafe { original(ssl, buffer, length) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_write_ex(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: usize,
    written: *mut usize,
) -> libc::c_int {
    let capture = dynamic_openssl_capture_enabled();
    let original = unsafe { interposed_ssl_write_ex(capture) };
    if capture {
        unsafe { ssl_write_ex_with(original, ssl, buffer, length, written) }
    } else {
        unsafe { original(ssl, buffer, length, written) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_read(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: libc::c_int,
) -> libc::c_int {
    let capture = dynamic_openssl_capture_enabled();
    let original = unsafe { interposed_ssl_read(capture) };
    if capture {
        unsafe { ssl_read_with(original, ssl, buffer, length) }
    } else {
        unsafe { original(ssl, buffer, length) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_read_ex(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: usize,
    read_bytes: *mut usize,
) -> libc::c_int {
    let capture = dynamic_openssl_capture_enabled();
    let original = unsafe { interposed_ssl_read_ex(capture) };
    if capture {
        unsafe { ssl_read_ex_with(original, ssl, buffer, length, read_bytes) }
    } else {
        unsafe { original(ssl, buffer, length, read_bytes) }
    }
}

fn openssl_ssl_read_ex_impl(address: usize) -> usize {
    let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, 16) };
    let wrapper_prefix = [0xf3, 0x0f, 0x1e, 0xfa, 0x55, 0x48, 0x89, 0xe5, 0xe8];
    if !bytes.starts_with(&wrapper_prefix) {
        return address;
    }
    let mut displacement = [0_u8; 4];
    displacement.copy_from_slice(&bytes[9..13]);
    let relative = i32::from_le_bytes(displacement) as isize;
    address.wrapping_add(13).wrapping_add_signed(relative)
}

fn replacement_for_symbol(symbol: &str) -> Result<usize, String> {
    match symbol {
        "SSL_write" => Ok(hook_ssl_write as *const () as usize),
        "SSL_write_ex" => Ok(hook_ssl_write_ex as *const () as usize),
        "SSL_read" => Ok(hook_ssl_read as *const () as usize),
        "SSL_read_ex" => Ok(hook_ssl_read_ex as *const () as usize),
        _ => Err(format!(
            "sync native rewrite does not support hook symbol yet: {symbol}"
        )),
    }
}

fn set_original(symbol: &str, trampoline: usize) -> Result<(), String> {
    match symbol {
        "SSL_write" => SSL_WRITE_ORIGINAL.store(trampoline, Ordering::Release),
        "SSL_write_ex" => SSL_WRITE_EX_ORIGINAL.store(trampoline, Ordering::Release),
        "SSL_read" => SSL_READ_ORIGINAL.store(trampoline, Ordering::Release),
        "SSL_read_ex" => SSL_READ_EX_ORIGINAL.store(trampoline, Ordering::Release),
        _ => return Err(format!("unknown original symbol slot: {symbol}")),
    }
    Ok(())
}

unsafe extern "C" fn hook_ssl_write(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: libc::c_int,
) -> libc::c_int {
    let original = unsafe { original_ssl_write() };
    unsafe { ssl_write_with(original, ssl, buffer, length) }
}

unsafe fn ssl_write_with(
    original: SslWriteFn,
    ssl: *mut c_void,
    buffer: *const c_void,
    length: libc::c_int,
) -> libc::c_int {
    if length <= 0 || buffer.is_null() {
        return unsafe { original(ssl, buffer, length) };
    }
    let Ok(length) = usize::try_from(length) else {
        return tls_write_error("SSL_write", "negative payload length");
    };
    let payload = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), length) };
    match decide_payload(
        PayloadDirection::Outbound,
        "SSL_write",
        ssl as usize,
        payload,
    ) {
        RuntimeAction::Allow => unsafe { original(ssl, buffer, length as libc::c_int) },
        RuntimeAction::Replace(replacement) => unsafe {
            original(
                ssl,
                replacement.as_ptr().cast::<c_void>(),
                length as libc::c_int,
            )
        },
        RuntimeAction::Block => tls_write_error("SSL_write", "processor blocked payload"),
    }
}

unsafe extern "C" fn hook_ssl_write_ex(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: usize,
    written: *mut usize,
) -> libc::c_int {
    let original = unsafe { original_ssl_write_ex() };
    unsafe { ssl_write_ex_with(original, ssl, buffer, length, written) }
}

unsafe fn ssl_write_ex_with(
    original: SslWriteExFn,
    ssl: *mut c_void,
    buffer: *const c_void,
    length: usize,
    written: *mut usize,
) -> libc::c_int {
    if length == 0 || buffer.is_null() {
        return unsafe { original(ssl, buffer, length, written) };
    }
    let payload = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), length) };
    match decide_payload(
        PayloadDirection::Outbound,
        "SSL_write_ex",
        ssl as usize,
        payload,
    ) {
        RuntimeAction::Allow => unsafe { original(ssl, buffer, length, written) },
        RuntimeAction::Replace(replacement) => unsafe {
            original(ssl, replacement.as_ptr().cast::<c_void>(), length, written)
        },
        RuntimeAction::Block => 0,
    }
}

unsafe extern "C" fn hook_ssl_read(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: libc::c_int,
) -> libc::c_int {
    let original = unsafe { original_ssl_read() };
    unsafe { ssl_read_with(original, ssl, buffer, length) }
}

unsafe fn ssl_read_with(
    original: SslReadFn,
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: libc::c_int,
) -> libc::c_int {
    let result = unsafe { original(ssl, buffer, length) };
    if result <= 0 || buffer.is_null() {
        return result;
    }
    let Ok(length) = usize::try_from(result) else {
        abort_runtime("SSL_read returned invalid length");
    };
    inbound_rewrite("SSL_read", ssl as usize, buffer.cast::<u8>(), length);
    result
}

unsafe extern "C" fn hook_ssl_read_ex(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: usize,
    read_bytes: *mut usize,
) -> libc::c_int {
    let original = unsafe { original_ssl_read_ex() };
    unsafe { ssl_read_ex_with(original, ssl, buffer, length, read_bytes) }
}

unsafe fn ssl_read_ex_with(
    original: SslReadExFn,
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: usize,
    read_bytes: *mut usize,
) -> libc::c_int {
    let result = unsafe { original(ssl, buffer, length, read_bytes) };
    if result != 1 || buffer.is_null() || read_bytes.is_null() {
        return result;
    }
    let completed = unsafe { *read_bytes };
    if completed == 0 {
        return result;
    }
    inbound_rewrite("SSL_read_ex", ssl as usize, buffer.cast::<u8>(), completed);
    result
}

fn dynamic_openssl_capture_enabled() -> bool {
    super::retry_initialize_after_loader_event();
    config::get().is_some_and(|config| config.provider() == "openssl" && !config.inline_hooks())
}

unsafe fn interposed_ssl_write(capture: bool) -> SslWriteFn {
    let address = interposed_symbol(
        capture,
        &SSL_WRITE_CONFIGURED,
        &SSL_WRITE_NEXT,
        b"SSL_write\0",
    );
    unsafe { std::mem::transmute(address) }
}

unsafe fn interposed_ssl_write_ex(capture: bool) -> SslWriteExFn {
    let address = interposed_symbol(
        capture,
        &SSL_WRITE_EX_CONFIGURED,
        &SSL_WRITE_EX_NEXT,
        b"SSL_write_ex\0",
    );
    unsafe { std::mem::transmute(address) }
}

unsafe fn interposed_ssl_read(capture: bool) -> SslReadFn {
    let address = interposed_symbol(capture, &SSL_READ_CONFIGURED, &SSL_READ_NEXT, b"SSL_read\0");
    unsafe { std::mem::transmute(address) }
}

unsafe fn interposed_ssl_read_ex(capture: bool) -> SslReadExFn {
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
    let Some(config) = config::get() else {
        abort_runtime("dynamic OpenSSL capture is active without runtime config");
    };
    let binary = CString::new(config.binary().as_os_str().as_bytes()).unwrap_or_else(|_| {
        abort_runtime(&format!(
            "configured OpenSSL binary path contains an interior NUL: {}",
            config.binary().display()
        ))
    });
    let handle = unsafe { super::loader::open_existing(binary.as_ptr()) };
    if handle.is_null() {
        abort_runtime(&format!(
            "configured OpenSSL binary is not loaded: {}",
            config.binary().display()
        ));
    }
    let address = unsafe { libc::dlsym(handle, symbol.as_ptr().cast()) } as usize;
    if address == 0 {
        abort_runtime(&format!(
            "configured OpenSSL binary {} does not export {}",
            config.binary().display(),
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
    let address = unsafe { libc::dlsym(libc::RTLD_NEXT, symbol.as_ptr().cast()) } as usize;
    if address == 0 {
        abort_runtime(&format!(
            "dynamic OpenSSL pass-through cannot resolve {}",
            symbol_name(symbol)
        ));
    }
    cache.store(address, Ordering::Release);
    address
}

fn symbol_name(symbol: &'static [u8]) -> &'static str {
    let raw = symbol.strip_suffix(b"\0").unwrap_or(symbol);
    std::str::from_utf8(raw).unwrap_or("<invalid>")
}

fn inbound_rewrite(symbol: &str, stream_key: usize, buffer: *mut u8, length: usize) {
    let payload = unsafe { std::slice::from_raw_parts(buffer, length) };
    match decide_payload(PayloadDirection::Inbound, symbol, stream_key, payload) {
        RuntimeAction::Allow => {}
        RuntimeAction::Replace(replacement) => unsafe {
            std::ptr::copy_nonoverlapping(replacement.as_ptr(), buffer, replacement.len());
        },
        RuntimeAction::Block => abort_runtime("inbound processor blocked payload"),
    }
}

unsafe fn original_ssl_write() -> SslWriteFn {
    let address = SSL_WRITE_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("SSL_write original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

unsafe fn original_ssl_write_ex() -> SslWriteExFn {
    let address = SSL_WRITE_EX_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("SSL_write_ex original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

unsafe fn original_ssl_read() -> SslReadFn {
    let address = SSL_READ_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("SSL_read original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

unsafe fn original_ssl_read_ex() -> SslReadExFn {
    let address = SSL_READ_EX_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("SSL_read_ex original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

fn tls_write_error(symbol: &str, reason: &str) -> libc::c_int {
    if config::get().is_some_and(|config| config.should_print_decision()) {
        output::event_line(&format!(
            "sync_decision: action=block direction=outbound symbol={symbol} reason={reason}\n"
        ));
    }
    -1
}

fn abort_runtime(message: &str) -> ! {
    output::error_line(&format!("tls_payload_probe_sync abort: {message}\n"));
    unsafe {
        libc::_exit(126);
    }
}
