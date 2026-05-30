//! Userspace retry for pathname pointers captured from syscall enter events.

use libc::{c_void, iovec, process_vm_readv};

const NUL_TERMINATOR_BYTES: usize = 1;
const NO_FLAGS: libc::c_ulong = 0;
const SINGLE_IOVEC_COUNT: libc::c_ulong = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UserPathRead {
    pub(super) value: String,
    pub(super) truncated: bool,
}

pub(super) fn read_process_path(pid: u32, pointer: u64, max_bytes: u32) -> Option<UserPathRead> {
    if pointer == 0 || max_bytes == 0 {
        return None;
    }
    let read_len = usize::try_from(max_bytes)
        .ok()?
        .checked_add(NUL_TERMINATOR_BYTES)?;
    let mut buffer = vec![0_u8; read_len];
    let local = iovec {
        iov_base: buffer.as_mut_ptr().cast::<c_void>(),
        iov_len: buffer.len(),
    };
    let remote = iovec {
        iov_base: pointer as *mut c_void,
        iov_len: buffer.len(),
    };

    let read = unsafe {
        process_vm_readv(
            pid as libc::pid_t,
            &local,
            SINGLE_IOVEC_COUNT,
            &remote,
            SINGLE_IOVEC_COUNT,
            NO_FLAGS,
        )
    };
    if read <= 0 {
        return None;
    }
    let read = usize::try_from(read).ok()?;
    let window = &buffer[..read];
    let (path_bytes, truncated) = match window.iter().position(|byte| *byte == 0) {
        Some(nul_index) => (&window[..nul_index], false),
        None => (window, true),
    };
    Some(UserPathRead {
        value: String::from_utf8_lossy(path_bytes).into_owned(),
        truncated,
    })
}
