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
    backend::install(target, replacement)
}
