//! Process event layouts and wire decoding.

use super::super::abi::{
    EVENT_HEADER_SIZE, EXEC_EVENT_FILENAME_FLAGS_OFFSET, EXEC_EVENT_FILENAME_OFFSET,
    EXEC_EVENT_FILENAME_SIZE_OFFSET, EXEC_FILENAME_ABI_MAX_BYTES, EXEC_FILENAME_FLAG_TRUNCATED,
    PROC_EXEC_EVENT_KIND, PROC_EXIT_EVENT_KIND, PROC_FORK_EVENT_KIND, PROC_SIGNAL_EVENT_KIND,
    PROCESS_EXEC_EVENT_SIZE, PROCESS_EXIT_EVENT_SIZE, PROCESS_FORK_EVENT_SIZE,
    PROCESS_SIGNAL_EVENT_SIZE,
};
use super::{
    KernelEventIdentity, KernelObservationEvent, KernelObservationPayload, KernelTypedEventHeader,
    read_i32, read_i64, read_u32, read_u64,
};
use crate::loader::LoaderError;
use collector_event::ExecFileIdentity;
use model_core::ids::TraceId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelTlsMappingEvent {
    pub common: super::KernelObservationCommon,
    pub file_identity: ExecFileIdentity,
    pub start: u64,
    pub end: u64,
}

impl KernelTlsMappingEvent {
    pub(super) fn decode(raw: &[u8]) -> Result<Self, LoaderError> {
        let header = KernelTypedEventHeader::decode(raw, super::TLS_MAPPING_EVENT_KIND, 108)?;
        let file_identity = Self::file_identity(raw, EVENT_HEADER_SIZE)
            .ok_or_else(|| LoaderError::new("decode_tls_mapping", "file identity is invalid"))?;
        let start = read_u64(raw, EVENT_HEADER_SIZE + 52).expect("event length checked");
        let end = read_u64(raw, EVENT_HEADER_SIZE + 60).expect("event length checked");
        if start >= end {
            return Err(LoaderError::new(
                "decode_tls_mapping",
                "VMA range is invalid",
            ));
        }
        Ok(Self {
            common: header.common(),
            file_identity,
            start,
            end,
        })
    }

    fn file_identity(raw: &[u8], offset: usize) -> Option<ExecFileIdentity> {
        (read_u32(raw, offset)? == 1).then(|| ExecFileIdentity {
            device_major: read_u32(raw, offset + 4).expect("event length checked"),
            device_minor: read_u32(raw, offset + 8).expect("event length checked"),
            inode: read_u64(raw, offset + 12).expect("event length checked"),
            size: read_u64(raw, offset + 20).expect("event length checked"),
            mtime_seconds: read_i64(raw, offset + 28).expect("event length checked"),
            ctime_seconds: read_i64(raw, offset + 36).expect("event length checked"),
            mtime_nanoseconds: read_u32(raw, offset + 44).expect("event length checked"),
            ctime_nanoseconds: read_u32(raw, offset + 48).expect("event length checked"),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelForkPayload {
    pub parent: KernelEventIdentity,
    pub attempt_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelExecPayload {
    pub filename: Option<KernelExecFilename>,
    pub exec_file_identity: Option<Box<ExecFileIdentity>>,
    pub attempt_id: u64,
    pub exec_attempt: Option<KernelProcessExecAttemptEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelExitPayload {
    pub exit_code: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelSignalPayload {
    pub signal_result: i32,
    pub signal: u32,
    pub target_kernel_tid: u32,
    pub target_group: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelExecFilename {
    pub path: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelProcessExecAttemptEvent {
    pub pid: u32,
    pub tid: u32,
    pub syscall: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub attempt_id: u64,
    pub pid_generation: u64,
    pub execveat_dirfd: i32,
    pub execveat_flags: u32,
    pub capture_flags: u32,
    pub host_pid: u32,
    pub host_tid: u32,
    pub path: String,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelProcessExecArgEvent {
    pub pid: u32,
    pub tid: u32,
    pub syscall: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub attempt_id: u64,
    pub pid_generation: u64,
    pub index: u32,
    pub capture_flags: u32,
    pub host_pid: u32,
    pub host_tid: u32,
    pub arg: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelProcessExecResultEvent {
    pub pid: u32,
    pub tid: u32,
    pub syscall: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub attempt_id: u64,
    pub pid_generation: u64,
    pub result: i64,
    pub host_pid: u32,
    pub host_tid: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelProcessForkAttemptEvent {
    pub pid: u32,
    pub tid: u32,
    pub syscall: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub attempt_id: u64,
    pub pid_generation: u64,
    pub flags: u64,
    pub clone3_args_ptr: u64,
    pub clone3_args_size: u64,
    pub capture_flags: u32,
    pub host_pid: u32,
    pub host_tid: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelProcessForkResultEvent {
    pub pid: u32,
    pub tid: u32,
    pub syscall: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub attempt_id: u64,
    pub pid_generation: u64,
    pub result: i64,
    pub host_pid: u32,
    pub host_tid: u32,
}

pub(super) fn decode_process_exec_attempt_event(
    raw: &[u8],
) -> Result<KernelProcessExecAttemptEvent, LoaderError> {
    const HEADER_SIZE: usize = 80;
    const PATH_MAX: usize = 4_096;
    if raw.len() < HEADER_SIZE {
        return Err(LoaderError::new(
            "decode_process_exec_attempt",
            format!(
                "event size {} is smaller than header {HEADER_SIZE}",
                raw.len()
            ),
        ));
    }
    let path_size = read_u32(raw, 56).expect("event length checked") as usize;
    if path_size > PATH_MAX || raw.len() != HEADER_SIZE + path_size {
        return Err(LoaderError::new(
            "decode_process_exec_attempt",
            format!("invalid path size {path_size} for event size {}", raw.len()),
        ));
    }
    Ok(KernelProcessExecAttemptEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        syscall: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        attempt_id: read_u64(raw, 32).expect("event length checked"),
        pid_generation: read_u64(raw, 40).expect("event length checked"),
        execveat_dirfd: read_i32(raw, 48).expect("event length checked"),
        execveat_flags: read_u32(raw, 52).expect("event length checked"),
        capture_flags: read_u32(raw, 68).expect("event length checked"),
        host_pid: read_u32(raw, 72).expect("event length checked"),
        host_tid: read_u32(raw, 76).expect("event length checked"),
        path: String::from_utf8_lossy(&raw[HEADER_SIZE..HEADER_SIZE + path_size]).into_owned(),
        argv: Vec::new(),
    })
}

pub(super) fn decode_process_exec_arg_event(
    raw: &[u8],
) -> Result<KernelProcessExecArgEvent, LoaderError> {
    const HEADER_SIZE: usize = 72;
    const ARG_MAX: usize = 4_096;
    if raw.len() < HEADER_SIZE {
        return Err(LoaderError::new(
            "decode_process_exec_arg",
            format!(
                "event size {} is smaller than header {HEADER_SIZE}",
                raw.len()
            ),
        ));
    }
    let arg_size = read_u32(raw, 52).expect("event length checked") as usize;
    if arg_size > ARG_MAX || raw.len() != HEADER_SIZE + arg_size {
        return Err(LoaderError::new(
            "decode_process_exec_arg",
            format!(
                "invalid argument size {arg_size} for event size {}",
                raw.len()
            ),
        ));
    }
    Ok(KernelProcessExecArgEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        syscall: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        attempt_id: read_u64(raw, 32).expect("event length checked"),
        pid_generation: read_u64(raw, 40).expect("event length checked"),
        index: read_u32(raw, 48).expect("event length checked"),
        capture_flags: read_u32(raw, 56).expect("event length checked"),
        host_pid: read_u32(raw, 60).expect("event length checked"),
        host_tid: read_u32(raw, 64).expect("event length checked"),
        arg: String::from_utf8_lossy(&raw[HEADER_SIZE..HEADER_SIZE + arg_size]).into_owned(),
    })
}

pub(super) fn decode_process_exec_result_event(
    raw: &[u8],
) -> Result<KernelProcessExecResultEvent, LoaderError> {
    const EVENT_SIZE: usize = 64;
    if raw.len() != EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_process_exec_result",
            format!("unexpected event size {}, expected {EVENT_SIZE}", raw.len()),
        ));
    }
    Ok(KernelProcessExecResultEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        syscall: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        attempt_id: read_u64(raw, 32).expect("event length checked"),
        pid_generation: read_u64(raw, 40).expect("event length checked"),
        result: read_i64(raw, 48).expect("event length checked"),
        host_pid: read_u32(raw, 56).expect("event length checked"),
        host_tid: read_u32(raw, 60).expect("event length checked"),
    })
}

pub(super) fn decode_process_fork_attempt_event(
    raw: &[u8],
) -> Result<KernelProcessForkAttemptEvent, LoaderError> {
    const EVENT_SIZE: usize = 88;
    if raw.len() != EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_process_fork_attempt",
            format!("unexpected event size {}, expected {EVENT_SIZE}", raw.len()),
        ));
    }
    Ok(KernelProcessForkAttemptEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        syscall: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        attempt_id: read_u64(raw, 32).expect("event length checked"),
        pid_generation: read_u64(raw, 40).expect("event length checked"),
        flags: read_u64(raw, 48).expect("event length checked"),
        clone3_args_ptr: read_u64(raw, 56).expect("event length checked"),
        clone3_args_size: read_u64(raw, 64).expect("event length checked"),
        capture_flags: read_u32(raw, 72).expect("event length checked"),
        host_pid: read_u32(raw, 76).expect("event length checked"),
        host_tid: read_u32(raw, 80).expect("event length checked"),
    })
}

pub(super) fn decode_process_fork_result_event(
    raw: &[u8],
) -> Result<KernelProcessForkResultEvent, LoaderError> {
    const EVENT_SIZE: usize = 64;
    if raw.len() != EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_process_fork_result",
            format!("unexpected event size {}, expected {EVENT_SIZE}", raw.len()),
        ));
    }
    Ok(KernelProcessForkResultEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        syscall: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        attempt_id: read_u64(raw, 32).expect("event length checked"),
        pid_generation: read_u64(raw, 40).expect("event length checked"),
        result: read_i64(raw, 48).expect("event length checked"),
        host_pid: read_u32(raw, 56).expect("event length checked"),
        host_tid: read_u32(raw, 60).expect("event length checked"),
    })
}

pub(super) fn decode_process_fork_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    let header =
        KernelTypedEventHeader::decode(raw, PROC_FORK_EVENT_KIND, PROCESS_FORK_EVENT_SIZE)?;
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::Fork(KernelForkPayload {
            parent: KernelEventIdentity {
                observer_namespace_tgid: read_u32(raw, EVENT_HEADER_SIZE)
                    .expect("event length checked"),
                kernel_tgid: read_u32(raw, EVENT_HEADER_SIZE + 4).expect("event length checked"),
                start_boottime_ns: read_u64(raw, EVENT_HEADER_SIZE + 8)
                    .expect("event length checked"),
            },
            attempt_id: read_u64(raw, EVENT_HEADER_SIZE + 16).expect("event length checked"),
        }),
    })
}

pub(super) fn decode_process_exec_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    let header =
        KernelTypedEventHeader::decode(raw, PROC_EXEC_EVENT_KIND, PROCESS_EXEC_EVENT_SIZE)?;
    let mut filename = None;
    let filename_size =
        read_u32(raw, EXEC_EVENT_FILENAME_SIZE_OFFSET).expect("event length checked");
    let filename_size = usize::try_from(filename_size).map_err(|error| {
        LoaderError::new(
            "decode_exec_event",
            format!("filename size overflow: {error}"),
        )
    })?;
    if filename_size > EXEC_FILENAME_ABI_MAX_BYTES {
        return Err(LoaderError::new(
            "decode_exec_event",
            format!(
                "exec filename size {} exceeds ABI maximum {}",
                filename_size, EXEC_FILENAME_ABI_MAX_BYTES
            ),
        ));
    }
    let flags = read_u32(raw, EXEC_EVENT_FILENAME_FLAGS_OFFSET).expect("event length checked");
    if filename_size > 0 {
        let filename_end = EXEC_EVENT_FILENAME_OFFSET + filename_size;
        filename = Some(KernelExecFilename {
            path: String::from_utf8_lossy(&raw[EXEC_EVENT_FILENAME_OFFSET..filename_end])
                .into_owned(),
            truncated: flags & EXEC_FILENAME_FLAG_TRUNCATED != 0,
        });
    }
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::Exec(KernelExecPayload {
            filename,
            exec_file_identity: KernelTlsMappingEvent::file_identity(
                raw,
                EXEC_EVENT_FILENAME_OFFSET + EXEC_FILENAME_ABI_MAX_BYTES,
            )
            .map(Box::new),
            attempt_id: read_u64(raw, EVENT_HEADER_SIZE).expect("event length checked"),
            exec_attempt: None,
        }),
    })
}

pub(super) fn decode_process_exit_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    const EXIT_CODE_VALID: u32 = 1;
    let header =
        KernelTypedEventHeader::decode(raw, PROC_EXIT_EVENT_KIND, PROCESS_EXIT_EVENT_SIZE)?;
    let exit_flags = read_u32(raw, EVENT_HEADER_SIZE + 4).expect("event length checked");
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::Exit(KernelExitPayload {
            exit_code: (exit_flags & EXIT_CODE_VALID != 0)
                .then(|| read_i32(raw, EVENT_HEADER_SIZE).expect("event length checked")),
        }),
    })
}

pub(super) fn decode_process_signal_event(
    raw: &[u8],
) -> Result<KernelObservationEvent, LoaderError> {
    let header =
        KernelTypedEventHeader::decode(raw, PROC_SIGNAL_EVENT_KIND, PROCESS_SIGNAL_EVENT_SIZE)?;
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::SignalGenerate(KernelSignalPayload {
            signal_result: read_i32(raw, EVENT_HEADER_SIZE).expect("event length checked"),
            signal: read_u32(raw, EVENT_HEADER_SIZE + 4).expect("event length checked"),
            target_kernel_tid: read_u32(raw, EVENT_HEADER_SIZE + 8).expect("event length checked"),
            target_group: read_u32(raw, EVENT_HEADER_SIZE + 12).expect("event length checked"),
        }),
    })
}
