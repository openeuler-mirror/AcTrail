//! Native inline hook installation.

use std::ffi::CStr;
use std::path::Path;

#[cfg(target_arch = "aarch64")]
#[path = "hook/aarch64.rs"]
mod backend;
#[cfg(target_arch = "x86_64")]
#[path = "hook/x86_64.rs"]
mod backend;
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[path = "hook/unsupported.rs"]
mod backend;

pub(super) fn install(target: usize, replacement: usize) -> Result<usize, String> {
    backend::install(target, replacement).map_err(|error| {
        format!("{error}; target_bytes={}", unsafe {
            render_target_bytes(target)
        })
    })
}

pub(super) fn installed_actrail_jump_target(target: usize) -> Option<usize> {
    let owner = backend::installed_jump_target(target)?;
    is_actrail_runtime_address(owner).then_some(owner)
}

fn is_actrail_runtime_address(address: usize) -> bool {
    let mut info = unsafe { std::mem::zeroed::<libc::Dl_info>() };
    let result = unsafe { libc::dladdr(address as *const libc::c_void, &mut info) };
    if result == 0 || info.dli_fname.is_null() {
        return false;
    }
    let path = unsafe { CStr::from_ptr(info.dli_fname) };
    Path::new(path.to_string_lossy().as_ref())
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("libactrail_tls_payload_probe_sync"))
}

unsafe fn render_target_bytes(target: usize) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(target as *const u8, 24) };
    let mut output = String::with_capacity(bytes.len() * 3);
    for (index, byte) in bytes.iter().enumerate() {
        if index != 0 {
            output.push(' ');
        }
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0f));
    }
    output
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => '?',
    }
}
