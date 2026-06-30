use std::ffi::c_void;

use tls_payload_core::PayloadDirection;

use crate::runtime::decision::{RuntimeAction, decide_payload};
use crate::runtime::{config, output};

use super::{
    OPENSSL_SSL_READ, OPENSSL_SSL_READ_EX, OPENSSL_SSL_WRITE, OPENSSL_SSL_WRITE_EX,
    OPENSSL_SSL_WRITE_EX2,
};

pub(in crate::runtime) type SslWriteFn =
    unsafe extern "C" fn(*mut c_void, *const c_void, libc::c_int) -> libc::c_int;
pub(in crate::runtime) type SslWriteExFn =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut usize) -> libc::c_int;
pub(in crate::runtime) type SslWriteEx2Fn =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, u64, *mut usize) -> libc::c_int;
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
        return tls_write_error(OPENSSL_SSL_WRITE, "negative payload length");
    };
    let payload = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), length) };
    match decide_payload(
        PayloadDirection::Outbound,
        OPENSSL_SSL_WRITE,
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
        RuntimeAction::Block => tls_write_error(OPENSSL_SSL_WRITE, "processor blocked payload"),
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
        OPENSSL_SSL_WRITE_EX,
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

pub(in crate::runtime) unsafe fn ssl_write_ex2_with(
    original: SslWriteEx2Fn,
    ssl: *mut c_void,
    buffer: *const c_void,
    length: usize,
    flags: u64,
    written: *mut usize,
) -> libc::c_int {
    if length == 0 || buffer.is_null() {
        return unsafe { original(ssl, buffer, length, flags, written) };
    }
    let payload = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), length) };
    match decide_payload(
        PayloadDirection::Outbound,
        OPENSSL_SSL_WRITE_EX2,
        ssl as usize,
        payload,
    ) {
        RuntimeAction::Allow => unsafe { original(ssl, buffer, length, flags, written) },
        RuntimeAction::Replace(replacement) => unsafe {
            original(
                ssl,
                replacement.as_ptr().cast::<c_void>(),
                length,
                flags,
                written,
            )
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
        abort_runtime(&format!("{OPENSSL_SSL_READ} returned invalid length"));
    };
    inbound_rewrite(OPENSSL_SSL_READ, ssl as usize, buffer.cast::<u8>(), length);
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
    inbound_rewrite(
        OPENSSL_SSL_READ_EX,
        ssl as usize,
        buffer.cast::<u8>(),
        completed,
    );
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
