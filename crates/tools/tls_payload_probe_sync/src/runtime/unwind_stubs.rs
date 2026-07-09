//! Fail-fast unwind symbols for the musl preload runtime.

use std::ffi::c_void;
use std::os::raw::c_int;

fn abort_unwind() -> ! {
    unsafe { libc::abort() }
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_Backtrace(_trace: *mut c_void, _argument: *mut c_void) -> c_int {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_DeleteException(_exception: *mut c_void) {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_ForcedUnwind(
    _exception: *mut c_void,
    _stop: *mut c_void,
    _stop_argument: *mut c_void,
) -> c_int {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_GetCFA(_context: *mut c_void) -> usize {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_GetDataRelBase(_context: *mut c_void) -> usize {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_GetGR(_context: *mut c_void, _index: c_int) -> usize {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_GetIP(_context: *mut c_void) -> usize {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_GetIPInfo(_context: *mut c_void, _ip_before: *mut c_int) -> usize {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_GetLanguageSpecificData(_context: *mut c_void) -> *mut c_void {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_GetRegionStart(_context: *mut c_void) -> usize {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_GetTextRelBase(_context: *mut c_void) -> usize {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_RaiseException(_exception: *mut c_void) -> c_int {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_Resume(_exception: *mut c_void) -> ! {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_Resume_or_Rethrow(_exception: *mut c_void) -> c_int {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_SetGR(_context: *mut c_void, _index: c_int, _value: usize) {
    abort_unwind()
}

#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_SetIP(_context: *mut c_void, _value: usize) {
    abort_unwind()
}
