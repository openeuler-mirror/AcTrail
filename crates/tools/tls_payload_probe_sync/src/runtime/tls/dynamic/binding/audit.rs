use std::ffi::{CStr, c_void};
use std::mem::MaybeUninit;
use std::os::raw::{c_char, c_uint};
use std::sync::atomic::{AtomicU8, Ordering};

use crate::runtime::tls::dynamic::core::{self, BindingSource, TlsFuncKind};
use crate::runtime::{self, loader};

const SUPPORTED_LAV_CURRENT: c_uint = 1;
const LA_FLG_BINDTO: c_uint = 0x01;
const LA_FLG_BINDFROM: c_uint = 0x02;

const OWN_RUNTIME_COOKIE: usize = 0xAC7A_11A0_0000_0001;

const NAMESPACE_UNKNOWN: u8 = 0;
const NAMESPACE_BASE: u8 = 1;
const NAMESPACE_NON_BASE: u8 = 2;

static NAMESPACE_STATE: AtomicU8 = AtomicU8::new(NAMESPACE_UNKNOWN);

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
    store_namespace_state(true);
    runtime::retry_initialize_after_loader_event();
    negotiate_audit_version(version)
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

pub(in crate::runtime) fn is_audit_namespace() -> Result<bool, String> {
    if let Some(is_audit) = cached_namespace_state() {
        return Ok(is_audit);
    }
    let is_audit = detect_current_namespace_is_audit()?;
    store_namespace_state(is_audit);
    Ok(is_audit)
}

fn cached_namespace_state() -> Option<bool> {
    match NAMESPACE_STATE.load(Ordering::Acquire) {
        NAMESPACE_BASE => Some(false),
        NAMESPACE_NON_BASE => Some(true),
        _ => None,
    }
}

fn store_namespace_state(is_audit: bool) {
    let state = if is_audit {
        NAMESPACE_NON_BASE
    } else {
        NAMESPACE_BASE
    };
    NAMESPACE_STATE.store(state, Ordering::Release);
}

fn detect_current_namespace_is_audit() -> Result<bool, String> {
    let lmid = current_runtime_lmid()?;
    Ok(lmid != libc::LM_ID_BASE as libc::Lmid_t)
}

fn current_runtime_lmid() -> Result<libc::Lmid_t, String> {
    let mut info = MaybeUninit::<libc::Dl_info>::zeroed();
    let address = current_runtime_lmid as *const () as *const c_void;
    let found = unsafe { libc::dladdr(address, info.as_mut_ptr()) };
    if found == 0 {
        return Err("resolve current runtime link-map: dladdr failed".to_string());
    }
    let info = unsafe { info.assume_init() };
    if info.dli_fname.is_null() {
        return Err("resolve current runtime link-map: dladdr returned no object path".to_string());
    }
    let handle = unsafe { loader::open_existing(info.dli_fname) };
    if handle.is_null() {
        let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy();
        return Err(format!(
            "resolve current runtime link-map: dlopen(RTLD_NOLOAD) failed for {path}"
        ));
    }
    let mut lmid = MaybeUninit::<libc::Lmid_t>::uninit();
    let result = unsafe {
        libc::dlinfo(
            handle,
            libc::RTLD_DI_LMID,
            lmid.as_mut_ptr().cast::<c_void>(),
        )
    };
    if result != 0 {
        let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy();
        return Err(format!(
            "resolve current runtime link-map: dlinfo(RTLD_DI_LMID) failed for {path}"
        ));
    }
    Ok(unsafe { lmid.assume_init() })
}

fn negotiate_audit_version(version: c_uint) -> c_uint {
    if version == 0 {
        0
    } else {
        version.min(SUPPORTED_LAV_CURRENT)
    }
}
