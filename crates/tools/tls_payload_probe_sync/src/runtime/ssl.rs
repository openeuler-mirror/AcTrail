//! OpenSSL/BoringSSL ABI hook handlers.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use tls_payload_core::PayloadDirection;

use crate::runtime::config::{self, HookPoint};
use crate::runtime::decision::{RuntimeAction, decide_payload};
use crate::runtime::{hook, maps, output, rustls};

static SSL_WRITE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_WRITE_EX_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SSL_READ_EX_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

type SslWriteFn = unsafe extern "C" fn(*mut c_void, *const c_void, libc::c_int) -> libc::c_int;
type SslWriteExFn =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut usize) -> libc::c_int;
type SslReadFn = unsafe extern "C" fn(*mut c_void, *mut c_void, libc::c_int) -> libc::c_int;
type SslReadExFn = unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut usize) -> libc::c_int;

pub(super) fn install_hooks(points: &[HookPoint]) -> Result<(), String> {
    let config = config::get().ok_or_else(|| "runtime config is not initialized".to_string())?;
    for point in points {
        let address = maps::runtime_address(config.binary(), point.file_offset)?;
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
