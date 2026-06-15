use std::ffi::c_void;

use tls_payload_core::PayloadDirection;

use crate::runtime::decision::{RuntimeAction, decide_payload};
use crate::runtime::{config, output};

pub(in crate::runtime) type SslWriteFn =
    unsafe extern "C" fn(*mut c_void, *const c_void, libc::c_int) -> libc::c_int;
pub(in crate::runtime) type SslWriteExFn =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut usize) -> libc::c_int;
pub(in crate::runtime) type SslReadFn =
    unsafe extern "C" fn(*mut c_void, *mut c_void, libc::c_int) -> libc::c_int;
pub(in crate::runtime) type SslReadExFn =
    unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut usize) -> libc::c_int;

pub(in crate::runtime) unsafe fn ssl_write_with(
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

pub(in crate::runtime) unsafe fn ssl_write_ex_with(
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

pub(in crate::runtime) unsafe fn ssl_read_with(
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

pub(in crate::runtime) unsafe fn ssl_read_ex_with(
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

pub(in crate::runtime) fn abort_runtime(message: &str) -> ! {
    output::error_line(&format!("tls_payload_probe_sync abort: {message}\n"));
    unsafe {
        libc::_exit(126);
    }
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

fn tls_write_error(symbol: &str, reason: &str) -> libc::c_int {
    if config::get().is_some_and(|config| config.should_print_decision()) {
        output::event_line(&format!(
            "sync_decision: action=block direction=outbound symbol={symbol} reason={reason}\n"
        ));
    }
    -1
}
