//! Process memory reads for seccomp user-notify capture windows.

use control_contract::reply::ControlError;

pub(crate) struct RemoteCString {
    pub(crate) value: Option<String>,
    pub(crate) truncated: bool,
}

pub(crate) fn read_c_string(
    pid: u32,
    remote_addr: u64,
    max_bytes: u32,
) -> Result<RemoteCString, ControlError> {
    if remote_addr == 0 {
        return Ok(RemoteCString {
            value: None,
            truncated: false,
        });
    }
    let max_bytes = usize::try_from(max_bytes)
        .map_err(|error| ControlError::new("seccomp_read", format!("size overflow: {error}")))?;
    let Some(bytes) = read_process_bytes(pid, remote_addr, max_bytes)? else {
        return Ok(RemoteCString {
            value: None,
            truncated: false,
        });
    };
    let end = bytes.iter().position(|byte| *byte == 0);
    let truncated = end.is_none();
    let value = match end {
        Some(end) => &bytes[..end],
        None => bytes.as_slice(),
    };
    let value = String::from_utf8(value.to_vec()).map_err(|error| {
        ControlError::new("seccomp_read", format!("path is not valid UTF-8: {error}"))
    })?;
    Ok(RemoteCString {
        value: Some(value),
        truncated,
    })
}

/// Errno from a capture-time read of a traced target that means the target already exited:
/// `ESRCH` from `process_vm_readv`, or `ENOENT` from a `/proc/<pid>` read. The seccomp capture
/// window is stale; the notification must be continued rather than crashing the daemon, so reads
/// surface this as `Ok(None)` ("nothing to capture") instead of a fatal error.
pub(crate) fn target_exited(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(errno) if errno == libc::ESRCH || errno == libc::ENOENT)
}

/// Returns `Ok(None)` when the target exited mid-capture (see [`target_exited`]).
pub(crate) fn read_linear_payload(
    pid: u32,
    remote_addr: u64,
    requested_size: u64,
    max_operation_bytes: u32,
) -> Result<Option<Vec<u8>>, ControlError> {
    let read_size = requested_size
        .min(u64::from(max_operation_bytes))
        .try_into()
        .map_err(|error| {
            ControlError::new("seccomp_read", format!("read size overflow: {error}"))
        })?;
    read_process_bytes(pid, remote_addr, read_size)
}

pub(crate) fn read_iovec_payload(
    pid: u32,
    remote_iovec_addr: u64,
    declared_iovec_count: usize,
    max_operation_bytes: u32,
) -> Result<Option<Vec<u8>>, ControlError> {
    if declared_iovec_count == 0 {
        return Ok(Some(Vec::new()));
    }
    let max_payload_bytes = usize::try_from(max_operation_bytes).map_err(|error| {
        ControlError::new(
            "seccomp_read",
            format!("max operation size overflow: {error}"),
        )
    })?;
    let iovec_size = std::mem::size_of::<libc::iovec>();
    let max_iovec_count = max_payload_bytes
        .checked_div(iovec_size)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| ControlError::new("seccomp_read", "iovec count overflow"))?;
    let iovec_count = declared_iovec_count.min(max_iovec_count);
    let table_size = iovec_count
        .checked_mul(iovec_size)
        .ok_or_else(|| ControlError::new("seccomp_read", "iovec table size overflow"))?;
    let Some(table) = read_process_bytes(pid, remote_iovec_addr, table_size)? else {
        return Ok(None);
    };
    if table.len() != table_size {
        return Err(ControlError::new(
            "seccomp_read",
            format!(
                "short iovec table read: expected {table_size}, read {}",
                table.len()
            ),
        ));
    }

    let mut bytes = Vec::new();
    for entry in table.chunks_exact(iovec_size) {
        if bytes.len() >= max_payload_bytes {
            break;
        }
        let iov_base = read_iovec_usize(entry, 0)? as u64;
        let iov_len = read_iovec_usize(entry, std::mem::size_of::<usize>())?;
        if iov_len == 0 {
            continue;
        }
        if iov_base == 0 {
            return Err(ControlError::new(
                "seccomp_read",
                "iovec entry has null base with non-zero length",
            ));
        }
        let remaining = max_payload_bytes - bytes.len();
        let read_len = iov_len.min(remaining);
        let Some(chunk) = read_process_bytes(pid, iov_base, read_len)? else {
            return Ok(None);
        };
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}

pub(crate) fn read_msghdr_iovec_payload(
    pid: u32,
    remote_msghdr_addr: u64,
    max_operation_bytes: u32,
) -> Result<Option<Vec<u8>>, ControlError> {
    let msghdr_size = std::mem::size_of::<libc::msghdr>();
    let Some(msghdr) = read_process_bytes(pid, remote_msghdr_addr, msghdr_size)? else {
        return Ok(None);
    };
    if msghdr.len() != msghdr_size {
        return Err(ControlError::new(
            "seccomp_read",
            format!(
                "short msghdr read: expected {msghdr_size}, read {}",
                msghdr.len()
            ),
        ));
    }
    let iov_addr = read_usize_at(
        &msghdr,
        std::mem::offset_of!(libc::msghdr, msg_iov),
        "msghdr",
    )? as u64;
    let iov_count = read_usize_at(
        &msghdr,
        std::mem::offset_of!(libc::msghdr, msg_iovlen),
        "msghdr",
    )?;
    if iov_count == 0 {
        return Ok(Some(Vec::new()));
    }
    if iov_addr == 0 {
        return Err(ControlError::new(
            "seccomp_read",
            "msghdr has null msg_iov with non-zero msg_iovlen",
        ));
    }
    read_iovec_payload(pid, iov_addr, iov_count, max_operation_bytes)
}

/// Returns `Ok(None)` when the target exited mid-capture (see [`target_exited`]).
pub(crate) fn read_process_bytes(
    pid: u32,
    remote_addr: u64,
    size: usize,
) -> Result<Option<Vec<u8>>, ControlError> {
    let mut bytes = vec![0_u8; size];
    let mut local = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut remote = libc::iovec {
        iov_base: remote_addr as usize as *mut libc::c_void,
        iov_len: size,
    };
    let read =
        unsafe { libc::process_vm_readv(pid as libc::pid_t, &mut local, 1, &mut remote, 1, 0) };
    if read < 0 {
        let error = std::io::Error::last_os_error();
        if target_exited(&error) {
            return Ok(None);
        }
        return Err(ControlError::new("seccomp_read", error.to_string()));
    }
    bytes.truncate(read as usize);
    Ok(Some(bytes))
}

fn read_iovec_usize(entry: &[u8], offset: usize) -> Result<usize, ControlError> {
    read_usize_at(entry, offset, "iovec table entry")
}

fn read_usize_at(bytes: &[u8], offset: usize, label: &str) -> Result<usize, ControlError> {
    let size = std::mem::size_of::<usize>();
    let bytes = bytes
        .get(offset..offset + size)
        .ok_or_else(|| ControlError::new("seccomp_read", format!("{label} is truncated")))?;
    Ok(usize::from_ne_bytes(
        bytes.try_into().expect("iovec usize width"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_exited_for_gone_errnos() {
        assert!(target_exited(&std::io::Error::from_raw_os_error(
            libc::ESRCH
        )));
        assert!(target_exited(&std::io::Error::from_raw_os_error(
            libc::ENOENT
        )));
        assert!(!target_exited(&std::io::Error::from_raw_os_error(
            libc::EFAULT
        )));
        assert!(!target_exited(&std::io::Error::from_raw_os_error(
            libc::EPERM
        )));
    }
}
