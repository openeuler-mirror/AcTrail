//! Process memory reads for seccomp user-notify capture windows.

use control_contract::reply::ControlError;

pub(crate) fn read_linear_payload(
    pid: u32,
    remote_addr: u64,
    requested_size: u64,
    max_operation_bytes: u32,
) -> Result<Vec<u8>, ControlError> {
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
) -> Result<Vec<u8>, ControlError> {
    if declared_iovec_count == 0 {
        return Ok(Vec::new());
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
    let table = read_process_bytes(pid, remote_iovec_addr, table_size)?;
    if table.len() != table_size {
        return Err(ControlError::new(
            "seccomp_read",
            format!(
                "short rustls iovec table read: expected {table_size}, read {}",
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
                "rustls iovec entry has null base with non-zero length",
            ));
        }
        let remaining = max_payload_bytes - bytes.len();
        let read_len = iov_len.min(remaining);
        let chunk = read_process_bytes(pid, iov_base, read_len)?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) fn read_process_bytes(
    pid: u32,
    remote_addr: u64,
    size: usize,
) -> Result<Vec<u8>, ControlError> {
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
        return Err(ControlError::new(
            "seccomp_read",
            std::io::Error::last_os_error().to_string(),
        ));
    }
    bytes.truncate(read as usize);
    Ok(bytes)
}

fn read_iovec_usize(entry: &[u8], offset: usize) -> Result<usize, ControlError> {
    let size = std::mem::size_of::<usize>();
    let bytes = entry.get(offset..offset + size).ok_or_else(|| {
        ControlError::new("seccomp_read", "rustls iovec table entry is truncated")
    })?;
    Ok(usize::from_ne_bytes(
        bytes.try_into().expect("iovec usize width"),
    ))
}
