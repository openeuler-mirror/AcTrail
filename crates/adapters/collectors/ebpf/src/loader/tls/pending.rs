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
    let tgid = read_tgid(tid)?;
    let namespace_key = ((tgid as u64) << 32) | u64::from(tid);
    let Some(host_key) = namespace_index
        .lookup(&namespace_key.to_ne_bytes(), MapFlags::ANY)
        .map_err(|error| LoaderError::new("lookup_pending_tls_payload_op", error.to_string()))?
        .map(|value| read_u64_value(&value))
        .transpose()?
    else {
        return Ok(None);
    };
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

fn read_tgid(tid: u32) -> Result<u32, LoaderError> {
    let status = std::fs::read_to_string(format!("/proc/{tid}/status"))
        .map_err(|error| LoaderError::new("lookup_pending_tls_payload_op", error.to_string()))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Tgid:"))
        .map(str::trim)
        .ok_or_else(|| LoaderError::new("lookup_pending_tls_payload_op", "missing Tgid"))?
        .parse::<u32>()
        .map_err(|error| LoaderError::new("lookup_pending_tls_payload_op", error.to_string()))
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
