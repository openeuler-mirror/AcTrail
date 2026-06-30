use std::ffi::c_void;

use crate::runtime::ssl;

use super::capture::{
    SslReadExFn, SslReadFn, SslWriteEx2Fn, SslWriteExFn, SslWriteFn, abort_runtime,
    ssl_read_ex_with, ssl_read_with, ssl_write_ex_with, ssl_write_ex2_with, ssl_write_with,
};
use super::{
    OPENSSL_SSL_READ, OPENSSL_SSL_READ_EX, OPENSSL_SSL_WRITE, OPENSSL_SSL_WRITE_EX,
    OPENSSL_SSL_WRITE_EX2, SLOT_COUNT, TlsFuncKind, real_symbol_for_slot,
};

type SslWriteEntry = unsafe extern "C" fn(*mut c_void, *const c_void, libc::c_int) -> libc::c_int;
type SslWriteExEntry =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut usize) -> libc::c_int;
type SslWriteEx2Entry =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, u64, *mut usize) -> libc::c_int;
type SslReadEntry = unsafe extern "C" fn(*mut c_void, *mut c_void, libc::c_int) -> libc::c_int;
type SslReadExEntry =
    unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut usize) -> libc::c_int;

pub(in crate::runtime) fn entry_for_slot(kind: TlsFuncKind, slot: usize) -> usize {
    match kind {
        TlsFuncKind::SslWrite => SSL_WRITE_ENTRIES[slot] as usize,
        TlsFuncKind::SslWriteEx => SSL_WRITE_EX_ENTRIES[slot] as usize,
        TlsFuncKind::SslWriteEx2 => SSL_WRITE_EX2_ENTRIES[slot] as usize,
        TlsFuncKind::SslRead => SSL_READ_ENTRIES[slot] as usize,
        TlsFuncKind::SslReadEx => SSL_READ_EX_ENTRIES[slot] as usize,
    }
}

pub(in crate::runtime) fn is_managed_entry(address: usize) -> bool {
    ssl::is_exported_ssl_entry(address)
        || SSL_WRITE_ENTRIES
            .iter()
            .any(|entry| *entry as usize == address)
        || SSL_WRITE_EX_ENTRIES
            .iter()
            .any(|entry| *entry as usize == address)
        || SSL_WRITE_EX2_ENTRIES
            .iter()
            .any(|entry| *entry as usize == address)
        || SSL_READ_ENTRIES
            .iter()
            .any(|entry| *entry as usize == address)
        || SSL_READ_EX_ENTRIES
            .iter()
            .any(|entry| *entry as usize == address)
}

unsafe extern "C" fn ssl_write_slot<const SLOT: usize>(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: libc::c_int,
) -> libc::c_int {
    let original = unsafe { real_ssl_write(SLOT) };
    unsafe { ssl_write_with(original, ssl, buffer, length) }
}

unsafe extern "C" fn ssl_write_ex_slot<const SLOT: usize>(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: usize,
    written: *mut usize,
) -> libc::c_int {
    let original = unsafe { real_ssl_write_ex(SLOT) };
    unsafe { ssl_write_ex_with(original, ssl, buffer, length, written) }
}

unsafe extern "C" fn ssl_write_ex2_slot<const SLOT: usize>(
    ssl: *mut c_void,
    buffer: *const c_void,
    length: usize,
    flags: u64,
    written: *mut usize,
) -> libc::c_int {
    let original = unsafe { real_ssl_write_ex2(SLOT) };
    unsafe { ssl_write_ex2_with(original, ssl, buffer, length, flags, written) }
}

unsafe extern "C" fn ssl_read_slot<const SLOT: usize>(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: libc::c_int,
) -> libc::c_int {
    let original = unsafe { real_ssl_read(SLOT) };
    unsafe { ssl_read_with(original, ssl, buffer, length) }
}

unsafe extern "C" fn ssl_read_ex_slot<const SLOT: usize>(
    ssl: *mut c_void,
    buffer: *mut c_void,
    length: usize,
    read_bytes: *mut usize,
) -> libc::c_int {
    let original = unsafe { real_ssl_read_ex(SLOT) };
    unsafe { ssl_read_ex_with(original, ssl, buffer, length, read_bytes) }
}

unsafe fn real_ssl_write(slot: usize) -> SslWriteFn {
    let address = real_symbol_for_slot(TlsFuncKind::SslWrite, slot).unwrap_or_else(|| {
        abort_runtime(&format!(
            "{OPENSSL_SSL_WRITE} dynamic slot has no real symbol"
        ))
    });
    unsafe { std::mem::transmute(address) }
}

unsafe fn real_ssl_write_ex(slot: usize) -> SslWriteExFn {
    let address = real_symbol_for_slot(TlsFuncKind::SslWriteEx, slot).unwrap_or_else(|| {
        abort_runtime(&format!(
            "{OPENSSL_SSL_WRITE_EX} dynamic slot has no real symbol"
        ))
    });
    unsafe { std::mem::transmute(address) }
}

unsafe fn real_ssl_write_ex2(slot: usize) -> SslWriteEx2Fn {
    let address = real_symbol_for_slot(TlsFuncKind::SslWriteEx2, slot).unwrap_or_else(|| {
        abort_runtime(&format!(
            "{OPENSSL_SSL_WRITE_EX2} dynamic slot has no real symbol"
        ))
    });
    unsafe { std::mem::transmute(address) }
}

unsafe fn real_ssl_read(slot: usize) -> SslReadFn {
    let address = real_symbol_for_slot(TlsFuncKind::SslRead, slot).unwrap_or_else(|| {
        abort_runtime(&format!(
            "{OPENSSL_SSL_READ} dynamic slot has no real symbol"
        ))
    });
    unsafe { std::mem::transmute(address) }
}

unsafe fn real_ssl_read_ex(slot: usize) -> SslReadExFn {
    let address = real_symbol_for_slot(TlsFuncKind::SslReadEx, slot).unwrap_or_else(|| {
        abort_runtime(&format!(
            "{OPENSSL_SSL_READ_EX} dynamic slot has no real symbol"
        ))
    });
    unsafe { std::mem::transmute(address) }
}

macro_rules! define_slot_entries {
    ($($slot:expr),+ $(,)?) => {
        static SSL_WRITE_ENTRIES: [SslWriteEntry; SLOT_COUNT] = [$(ssl_write_slot::<$slot>),+];
        static SSL_WRITE_EX_ENTRIES: [SslWriteExEntry; SLOT_COUNT] = [$(ssl_write_ex_slot::<$slot>),+];
        static SSL_WRITE_EX2_ENTRIES: [SslWriteEx2Entry; SLOT_COUNT] = [$(ssl_write_ex2_slot::<$slot>),+];
        static SSL_READ_ENTRIES: [SslReadEntry; SLOT_COUNT] = [$(ssl_read_slot::<$slot>),+];
        static SSL_READ_EX_ENTRIES: [SslReadExEntry; SLOT_COUNT] = [$(ssl_read_ex_slot::<$slot>),+];
    };
}

define_slot_entries!(
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
    74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97,
    98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
    117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127,
);
