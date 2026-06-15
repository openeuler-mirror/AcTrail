//! Native inline hook installation.

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
