use std::ffi::{CStr, c_void};
use std::os::raw::{c_char, c_uint};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::runtime;
use crate::runtime::tls::dynamic::core::{self, BindingSource, TlsFuncKind};

const LAV_CURRENT: c_uint = 2;
const LA_FLG_BINDTO: c_uint = 0x01;
const LA_FLG_BINDFROM: c_uint = 0x02;

const OWN_RUNTIME_COOKIE: usize = 0xAC7A_11A0_0000_0001;

static AUDIT_NAMESPACE: AtomicBool = AtomicBool::new(false);

#[repr(C)]
pub(in crate::runtime) struct Elf64Sym {
    st_name: u32,
    st_info: u8,
    st_other: u8,
    st_shndx: u16,
    st_value: u64,
    st_size: u64,
}

#[repr(C)]
struct LinkMap {
    l_addr: usize,
    l_name: *const c_char,
    l_ld: *mut c_void,
    l_next: *mut c_void,
    l_prev: *mut c_void,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn la_version(version: c_uint) -> c_uint {
    AUDIT_NAMESPACE.store(true, Ordering::Release);
    runtime::retry_initialize_after_loader_event();
    if version >= LAV_CURRENT {
        LAV_CURRENT
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn la_objopen(
    map: *mut c_void,
    _lmid: libc::Lmid_t,
    cookie: *mut usize,
) -> c_uint {
    runtime::retry_initialize_after_loader_event();
    if own_runtime_object(map.cast()) && !cookie.is_null() {
        unsafe {
            *cookie = OWN_RUNTIME_COOKIE;
        }
    }
    LA_FLG_BINDTO | LA_FLG_BINDFROM
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn la_symbind64(
    sym: *mut Elf64Sym,
    _ndx: c_uint,
    _refcook: *mut usize,
    defcook: *mut usize,
    _flags: *mut c_uint,
    symname: *const c_char,
) -> usize {
    let real = unsafe { sym.as_ref().map(|sym| sym.st_value as usize).unwrap_or(0) };
    if audit_cookie(defcook) == OWN_RUNTIME_COOKIE || audit_cookie(_refcook) == OWN_RUNTIME_COOKIE {
        return real;
    }
    let Some(kind) = TlsFuncKind::from_c_symbol(symname) else {
        return real;
    };
    core::get_or_create_bound_wrapper(kind, real, BindingSource::Audit).unwrap_or(real)
}

fn audit_cookie(cookie: *mut usize) -> usize {
    if cookie.is_null() {
        return 0;
    }
    unsafe { *cookie }
}

fn own_runtime_object(map: *mut LinkMap) -> bool {
    let Some(map) = (unsafe { map.as_ref() }) else {
        return false;
    };
    if map.l_name.is_null() {
        return false;
    }
    let Ok(path) = (unsafe { CStr::from_ptr(map.l_name) }).to_str() else {
        return false;
    };
    path.rsplit('/')
        .next()
        .is_some_and(|name| name == "libactrail_tls_payload_probe_sync.so")
}

pub(in crate::runtime) fn is_audit_namespace() -> bool {
    AUDIT_NAMESPACE.load(Ordering::Acquire)
}
