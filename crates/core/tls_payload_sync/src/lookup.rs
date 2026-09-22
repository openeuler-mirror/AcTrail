//! Typed plan lookup request/response exchange for the preloaded runtime.
use crate::runtime::{DeadlineStream, Endpoint};
use crate::{FrameCodec, RuntimePlanDescriptor, SyncError, SyncResult};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanLookupRequest {
    pub binary: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanLookupResponse {
    Found(RuntimePlanDescriptor),
    Unsupported { reason: String },
}

pub fn lookup_runtime_plan(
    socket_path: &Path,
    binary: &Path,
    max_frame_bytes: usize,
    timeout: Duration,
) -> SyncResult<PlanLookupResponse> {
    if timeout.is_zero() {
        return Err(SyncError::new("TLS plan lookup timeout must be positive"));
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| SyncError::new("TLS plan lookup timeout exceeds the supported duration"))?;
    let stream = Endpoint::Path(socket_path.to_path_buf()).connect(deadline)?;
    let mut stream = DeadlineStream::new(stream, max_frame_bytes);
    stream.set_deadline(deadline);
    FrameCodec::write_lookup_request(
        &mut stream,
        &PlanLookupRequest {
            binary: binary.to_path_buf(),
        },
        max_frame_bytes,
    )?;
    FrameCodec::read_lookup_response(&mut stream, max_frame_bytes)
}
