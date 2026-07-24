//! Pending TLS payload lookup across kernel and namespace PID domains.

use libbpf_rs::{MapCore, MapFlags, MapHandle};
use model_core::ids::TraceId;

use crate::loader::LoaderError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingTlsPayloadOp {
    pub trace_id: TraceId,
    pub operation_id: u64,
    pub tgid: u32,
    pub tid: u32,
    pub stream_key: u64,
    pub buffer_ptr: u64,
    pub requested_size: u64,
    pub direction: u32,
    pub symbol: u32,
    pub library: u32,
    pub capture_state: u32,
    pub pid_generation: u64,
}

pub(crate) fn lookup_pending_payload_op(
    namespace_index: &MapHandle,
    operations: &MapHandle,
    tid: u32,
) -> Result<Option<PendingTlsPayloadOp>, LoaderError> {
    let Some(tgid) = read_tgid(tid)? else {
        return Ok(None);
    };
    let host_pid_tgid = ((tgid as u64) << 32) | u64::from(tid);
    let host_key = namespace_index
        .lookup(&host_pid_tgid.to_ne_bytes(), MapFlags::ANY)
        .map_err(|error| LoaderError::new("lookup_pending_tls_payload_op", error.to_string()))?
        .map(|value| read_u64_value(&value))
        .transpose()?
        .unwrap_or(host_pid_tgid);
    operations
        .lookup(&host_key.to_ne_bytes(), MapFlags::ANY)
        .map_err(|error| LoaderError::new("lookup_pending_tls_payload_op", error.to_string()))?
        .map(|value| pending_tls_payload_op_from_bytes(tgid, tid, &value))
        .transpose()
}

fn pending_tls_payload_op_from_bytes(
    tgid: u32,
    tid: u32,
    value: &[u8],
) -> Result<PendingTlsPayloadOp, LoaderError> {
    Ok(PendingTlsPayloadOp {
        trace_id: TraceId::new(read_u64(value, 0)?),
        operation_id: read_u64(value, 8)?,
        tgid,
        tid,
        stream_key: read_u64(value, 16)?,
        buffer_ptr: read_u64(value, 24)?,
        requested_size: read_u64(value, 32)?,
        pid_generation: read_u64(value, 48)?,
        direction: read_u32(value, 56)?,
        symbol: read_u32(value, 60)?,
        library: read_u32(value, 64)?,
        capture_state: read_u32(value, 68)?,
    })
}

/// Errno from a `/proc` read of the traced thread meaning it already exited: `ENOENT` (the entry
/// was gone before the open) or `ESRCH` (the task was reaped while the status file was being read,
/// which surfaces as an `Uncategorized` io error kind rather than `NotFound`).
fn target_exited(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(errno) if errno == libc::ENOENT || errno == libc::ESRCH)
}

/// Reads the thread group id of `tid`. Returns `Ok(None)` when the target thread has already
/// exited: the seccomp capture window is stale, so the caller must treat it as "no pending op"
/// rather than crash the daemon. The thread can vanish at two points — `ENOENT` if its `/proc`
/// entry is already gone before the open, or `ESRCH` if it exits while the status file is being
/// read — so both errnos are matched (the latter surfaces as an `Uncategorized` io error kind).
fn read_tgid(tid: u32) -> Result<Option<u32>, LoaderError> {
    let status = match std::fs::read_to_string(format!("/proc/{tid}/status")) {
        Ok(status) => status,
        Err(error) if target_exited(&error) => return Ok(None),
        Err(error) => {
            return Err(LoaderError::new(
                "lookup_pending_tls_payload_op",
                error.to_string(),
            ));
        }
    };
    let tgid = status
        .lines()
        .find_map(|line| line.strip_prefix("Tgid:"))
        .map(str::trim)
        .ok_or_else(|| LoaderError::new("lookup_pending_tls_payload_op", "missing Tgid"))?
        .parse::<u32>()
        .map_err(|error| LoaderError::new("lookup_pending_tls_payload_op", error.to_string()))?;
    Ok(Some(tgid))
}

fn read_u64_value(value: &[u8]) -> Result<u64, LoaderError> {
    read_u64(value, 0)
}

fn read_u64(value: &[u8], offset: usize) -> Result<u64, LoaderError> {
    value
        .get(offset..offset + std::mem::size_of::<u64>())
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_ne_bytes)
        .ok_or_else(|| unexpected_value_size(value))
}

fn read_u32(value: &[u8], offset: usize) -> Result<u32, LoaderError> {
    value
        .get(offset..offset + std::mem::size_of::<u32>())
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_ne_bytes)
        .ok_or_else(|| unexpected_value_size(value))
}

fn unexpected_value_size(value: &[u8]) -> LoaderError {
    LoaderError::new(
        "lookup_pending_tls_payload_op",
        format!("unexpected pending TLS map value size {}", value.len()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_exited_covers_proc_read_race_errnos() {
        // ENOENT: /proc entry gone before open. ESRCH: thread reaped mid-read.
        assert!(target_exited(&std::io::Error::from_raw_os_error(
            libc::ENOENT
        )));
        assert!(target_exited(&std::io::Error::from_raw_os_error(
            libc::ESRCH
        )));
        assert!(!target_exited(&std::io::Error::from_raw_os_error(
            libc::EACCES
        )));
    }
}
