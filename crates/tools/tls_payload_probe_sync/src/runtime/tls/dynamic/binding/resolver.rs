use std::cell::Cell;
use std::ffi::{CStr, c_void};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::runtime::tls::dynamic::core::BindingSource;

use super::common::maybe_bound_wrapper;

type DlsymFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
type DlvsymFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *mut c_void;

static REAL_DLSYM: AtomicUsize = AtomicUsize::new(0);
static REAL_DLVSYM: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static RESOLVER_GUARD: Cell<bool> = const { Cell::new(false) };
}

pub(in crate::runtime) unsafe fn real_dlsym(
    handle: *mut c_void,
    symbol: *const c_char,
) -> *mut c_void {
    let Some(real) = real_dlsym_fn() else {
        return std::ptr::null_mut();
    };
    unsafe { real(handle, symbol) }
}

pub(in crate::runtime) fn libc_symbol(symbol: &str) -> Option<usize> {
    resolve_libc_symbol(symbol)
}

pub(in crate::runtime) unsafe fn real_dlvsym(
    handle: *mut c_void,
    symbol: *const c_char,
    version: *const c_char,
) -> *mut c_void {
    let Some(real) = real_dlvsym_fn() else {
        return std::ptr::null_mut();
    };
    unsafe { real(handle, symbol, version) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    RESOLVER_GUARD.with(|guard| {
        if guard.get() {
            return unsafe { real_dlsym(handle, symbol) };
        }
        guard.set(true);
        let real = unsafe { real_dlsym(handle, symbol) };
        let resolved = maybe_bound_wrapper(symbol, real, BindingSource::Resolver);
        guard.set(false);
        resolved
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlvsym(
    handle: *mut c_void,
    symbol: *const c_char,
    version: *const c_char,
) -> *mut c_void {
    RESOLVER_GUARD.with(|guard| {
        if guard.get() {
            return unsafe { real_dlvsym(handle, symbol, version) };
        }
        guard.set(true);
        let real = unsafe { real_dlvsym(handle, symbol, version) };
        let resolved = maybe_bound_wrapper(symbol, real, BindingSource::Resolver);
        guard.set(false);
        resolved
    })
}

fn real_dlsym_fn() -> Option<DlsymFn> {
    let cached = REAL_DLSYM.load(Ordering::Acquire);
    if cached != 0 {
        return Some(unsafe { std::mem::transmute::<usize, DlsymFn>(cached) });
    }
    let address = resolve_libc_symbol("dlsym")?;
    REAL_DLSYM.store(address, Ordering::Release);
    Some(unsafe { std::mem::transmute::<usize, DlsymFn>(address) })
}

fn real_dlvsym_fn() -> Option<DlvsymFn> {
    let cached = REAL_DLVSYM.load(Ordering::Acquire);
    if cached != 0 {
        return Some(unsafe { std::mem::transmute::<usize, DlvsymFn>(cached) });
    }
    let address = resolve_libc_symbol("dlvsym")?;
    REAL_DLVSYM.store(address, Ordering::Release);
    Some(unsafe { std::mem::transmute::<usize, DlvsymFn>(address) })
}

fn resolve_libc_symbol(symbol: &str) -> Option<usize> {
    let mapping = libc_mapping()?;
    let value = elf_dynsym_value(&mapping.path, symbol)?;
    Some(mapping.load_bias.wrapping_add(usize::try_from(value).ok()?))
}

struct LibcMapping {
    path: PathBuf,
    load_bias: usize,
}

fn libc_mapping() -> Option<LibcMapping> {
    if let Some(mapping) = libc_mapping_from_phdr() {
        return Some(mapping);
    }
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    for line in maps.lines() {
        let mut fields = line.split_whitespace();
        let range = fields.next()?;
        let _perms = fields.next()?;
        let offset = usize::from_str_radix(fields.next()?, 16).ok()?;
        let _dev = fields.next()?;
        let _inode = fields.next()?;
        let path = PathBuf::from(fields.next()?);
        if !is_libc_path(&path) {
            continue;
        }
        let start = usize::from_str_radix(range.split_once('-')?.0, 16).ok()?;
        return Some(LibcMapping {
            path,
            load_bias: start.wrapping_sub(offset),
        });
    }
    None
}

fn libc_mapping_from_phdr() -> Option<LibcMapping> {
    let mut mapping = None;
    unsafe {
        libc::dl_iterate_phdr(
            Some(collect_libc_mapping),
            (&mut mapping as *mut Option<LibcMapping>).cast(),
        );
    }
    mapping
}

unsafe extern "C" fn collect_libc_mapping(
    info: *mut libc::dl_phdr_info,
    _size: usize,
    data: *mut c_void,
) -> libc::c_int {
    let Some(info) = (unsafe { info.as_ref() }) else {
        return 0;
    };
    if info.dlpi_name.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(info.dlpi_name) };
    if path.to_bytes().is_empty() {
        return 0;
    }
    let path = PathBuf::from(path.to_string_lossy().as_ref());
    if !is_libc_path(&path) {
        return 0;
    }
    let output = unsafe { &mut *(data.cast::<Option<LibcMapping>>()) };
    *output = Some(LibcMapping {
        path,
        load_bias: info.dlpi_addr as usize,
    });
    1
}

fn is_libc_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == "libc.so.6"
                || name == "libc.so"
                || name.starts_with("libc-")
                || name.starts_with("libc.musl-")
                || (name.starts_with("ld-musl-") && name.ends_with(".so.1"))
        })
        && path.is_file()
}

fn elf_dynsym_value(path: &Path, symbol: &str) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.get(0..4)? != b"\x7fELF" || *bytes.get(4)? != 2 || *bytes.get(5)? != 1 {
        return None;
    }
    let shoff = read_u64(&bytes, 0x28)? as usize;
    let shentsize = read_u16(&bytes, 0x3a)? as usize;
    let shnum = read_u16(&bytes, 0x3c)? as usize;
    let shstrndx = read_u16(&bytes, 0x3e)? as usize;
    if shentsize == 0 || shnum == 0 || shstrndx >= shnum {
        return None;
    }
    let shstr = section(&bytes, shoff, shentsize, shstrndx)?;
    let shstr_data = bytes.get(shstr.offset..shstr.offset.checked_add(shstr.size)?)?;
    let mut dynsym = None;
    let mut dynstr = None;
    for index in 0..shnum {
        let header = section(&bytes, shoff, shentsize, index)?;
        let name = c_string_at(shstr_data, header.name)?;
        match name.to_bytes() {
            b".dynsym" => dynsym = Some(header),
            b".dynstr" => dynstr = Some(header),
            _ => {}
        }
    }
    let dynsym = dynsym?;
    let dynstr = dynstr?;
    let strings = bytes.get(dynstr.offset..dynstr.offset.checked_add(dynstr.size)?)?;
    let entry_size = if dynsym.entsize == 0 {
        24
    } else {
        dynsym.entsize
    };
    let entries = dynsym.size / entry_size;
    for index in 0..entries {
        let offset = dynsym.offset.checked_add(index.checked_mul(entry_size)?)?;
        let st_name = read_u32(&bytes, offset)? as usize;
        let st_value = read_u64(&bytes, offset + 8)?;
        if st_value == 0 {
            continue;
        }
        let name = c_string_at(strings, st_name)?;
        if name.to_bytes() == symbol.as_bytes() {
            return Some(st_value);
        }
    }
    None
}

#[derive(Clone, Copy)]
struct Section {
    name: usize,
    offset: usize,
    size: usize,
    entsize: usize,
}

fn section(bytes: &[u8], shoff: usize, shentsize: usize, index: usize) -> Option<Section> {
    let offset = shoff.checked_add(index.checked_mul(shentsize)?)?;
    Some(Section {
        name: read_u32(bytes, offset)? as usize,
        offset: read_u64(bytes, offset + 0x18)? as usize,
        size: read_u64(bytes, offset + 0x20)? as usize,
        entsize: read_u64(bytes, offset + 0x38)? as usize,
    })
}

fn c_string_at(bytes: &[u8], offset: usize) -> Option<&CStr> {
    let rest = bytes.get(offset..)?;
    let len = rest.iter().position(|byte| *byte == 0)?;
    CStr::from_bytes_with_nul(rest.get(..=len)?).ok()
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes(raw.try_into().ok()?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes(raw.try_into().ok()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let raw = bytes.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes(raw.try_into().ok()?))
}
