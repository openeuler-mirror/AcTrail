//! Kernel process identity and observer thread delivery coordinate for socket operations.
//! The listener supplies only observer TIDs; the bound kernel TGID and generation
//! remain authoritative. No reverse thread index is required.

use ebpf_collector::SocketPayloadCompletion;
use model_core::ids::TraceId;

use super::request::SocketReadRequest;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct SocketCaptureKey {
    pub(super) trace_id: TraceId,
    pub(super) pid_generation: u64,
    pub(super) socket_generation: u32,
    pub(super) kernel_tgid: u32,
    pub(super) observer_tid: u32,
    pub(super) fd: u32,
    pub(super) syscall: u32,
    pub(super) buffer_ptr: u64,
    pub(super) requested_size: u64,
}

impl SocketCaptureKey {
    pub(super) fn from_request(
        trace_id: TraceId,
        pid_generation: u64,
        socket_generation: u32,
        kernel_tgid: u32,
        observer_tid: u32,
        request: &SocketReadRequest,
    ) -> Self {
        Self {
            trace_id,
            pid_generation,
            socket_generation,
            kernel_tgid,
            observer_tid,
            fd: request.fd,
            syscall: request.syscall,
            buffer_ptr: request.key_buffer_ptr,
            requested_size: request.key_requested_size,
        }
    }

    pub(super) fn from_completion(completion: &SocketPayloadCompletion) -> Self {
        Self {
            trace_id: completion.trace_id,
            pid_generation: completion.pid_generation,
            socket_generation: completion.socket_generation,
            kernel_tgid: completion.kernel_tgid,
            observer_tid: completion.observer_tid,
            fd: completion.fd,
            syscall: completion.syscall,
            buffer_ptr: completion.buffer_ptr,
            requested_size: completion.requested_size,
        }
    }
}
