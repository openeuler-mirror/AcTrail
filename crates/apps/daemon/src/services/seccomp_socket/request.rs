//! Seccomp socket syscall request decoding.

use std::path::PathBuf;

use control_contract::reply::ControlError;
use ebpf_collector::{
    SOCKET_PAYLOAD_SYSCALL_SENDMSG, SOCKET_PAYLOAD_SYSCALL_SENDTO, SOCKET_PAYLOAD_SYSCALL_WRITE,
    SOCKET_PAYLOAD_SYSCALL_WRITEV,
};

use crate::services::seccomp_notify::{
    read_iovec_payload, read_linear_payload, read_msghdr_iovec_payload, target_exited,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SocketReadRequest {
    pub(super) fd: u32,
    pub(super) syscall: u32,
    pub(super) key_buffer_ptr: u64,
    pub(super) key_requested_size: u64,
    source: SocketPayloadSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SocketPayloadSource {
    Linear {
        buffer_ptr: u64,
        requested_size: u64,
    },
    Iovec {
        iovec_ptr: u64,
        iovec_count: usize,
    },
    MsgHdr {
        msghdr_ptr: u64,
    },
}

impl SocketReadRequest {
    pub(super) fn from_notification(
        notification: &libc::seccomp_notif,
    ) -> Result<Option<Self>, ControlError> {
        let syscall = syscall_from_notification(notification)?;
        let Some(syscall) = syscall else {
            return Ok(None);
        };
        let fd = u32::try_from(notification.data.args[0]).map_err(|error| {
            ControlError::new("seccomp_socket_args", format!("fd overflow: {error}"))
        })?;
        let key_buffer_ptr = notification.data.args[1];
        let key_requested_size = key_requested_size(syscall, notification);
        let source = match syscall {
            SOCKET_PAYLOAD_SYSCALL_WRITE | SOCKET_PAYLOAD_SYSCALL_SENDTO => {
                SocketPayloadSource::Linear {
                    buffer_ptr: notification.data.args[1],
                    requested_size: notification.data.args[2],
                }
            }
            SOCKET_PAYLOAD_SYSCALL_WRITEV => SocketPayloadSource::Iovec {
                iovec_ptr: notification.data.args[1],
                iovec_count: usize::try_from(notification.data.args[2]).map_err(|error| {
                    ControlError::new("seccomp_socket_args", format!("iovcnt overflow: {error}"))
                })?,
            },
            SOCKET_PAYLOAD_SYSCALL_SENDMSG => SocketPayloadSource::MsgHdr {
                msghdr_ptr: notification.data.args[1],
            },
            other => {
                return Err(ControlError::new(
                    "seccomp_socket_args",
                    format!("unsupported socket payload syscall {other}"),
                ));
            }
        };
        Ok(Some(Self {
            fd,
            syscall,
            key_buffer_ptr,
            key_requested_size,
            source,
        }))
    }

    pub(super) fn skip_small_linear_payload(&self, max_segment_bytes: u32) -> bool {
        match self.source {
            SocketPayloadSource::Linear { requested_size, .. } => {
                requested_size <= u64::from(max_segment_bytes)
            }
            SocketPayloadSource::Iovec { .. } | SocketPayloadSource::MsgHdr { .. } => false,
        }
    }

    pub(super) fn requires_socket_fd_check(&self) -> bool {
        matches!(
            self.syscall,
            SOCKET_PAYLOAD_SYSCALL_WRITE | SOCKET_PAYLOAD_SYSCALL_WRITEV
        )
    }

    pub(super) fn read_size_hint(&self) -> u64 {
        match self.source {
            SocketPayloadSource::Linear { requested_size, .. } => requested_size,
            SocketPayloadSource::Iovec { .. } | SocketPayloadSource::MsgHdr { .. } => {
                u64::from(u32::MAX)
            }
        }
    }

    pub(super) fn read_payload(
        &self,
        pid: u32,
        requested_size: u64,
        max_operation_bytes: u32,
    ) -> Result<Option<Vec<u8>>, ControlError> {
        let max_bytes_u64 = requested_size.min(u64::from(max_operation_bytes));
        let max_bytes_u32 = u32::try_from(max_bytes_u64).map_err(|error| {
            ControlError::new(
                "seccomp_socket_read",
                format!("read size overflow: {error}"),
            )
        })?;
        match self.source {
            SocketPayloadSource::Linear {
                buffer_ptr,
                requested_size,
            } => read_linear_payload(
                pid,
                buffer_ptr,
                requested_size.min(max_bytes_u64),
                max_operation_bytes,
            ),
            SocketPayloadSource::Iovec {
                iovec_ptr,
                iovec_count,
            } => read_iovec_payload(pid, iovec_ptr, iovec_count, max_bytes_u32),
            SocketPayloadSource::MsgHdr { msghdr_ptr } => {
                read_msghdr_iovec_payload(pid, msghdr_ptr, max_bytes_u32)
            }
        }
    }
}

pub(super) fn fd_is_socket(pid: u32, fd: u32) -> Result<bool, ControlError> {
    let path = PathBuf::from(format!("/proc/{pid}/fd/{fd}"));
    let target = match std::fs::read_link(&path) {
        Ok(target) => target,
        // Target exited mid-capture; report "not a socket" so callers skip this stale notification.
        Err(error) if target_exited(&error) => return Ok(false),
        Err(error) => {
            return Err(ControlError::new(
                "seccomp_socket_fd",
                format!("readlink {}: {error}", path.display()),
            ));
        }
    };
    Ok(target.to_string_lossy().starts_with("socket:["))
}

/// Returns `Ok(None)` when the target exited mid-capture (its `/proc` entry is gone).
pub(super) fn tgid_from_status(tid: u32) -> Result<Option<u32>, ControlError> {
    let status = match std::fs::read_to_string(format!("/proc/{tid}/status")) {
        Ok(status) => status,
        Err(error) if target_exited(&error) => return Ok(None),
        Err(error) => return Err(ControlError::new("seccomp_socket_tgid", error.to_string())),
    };
    let tgid = status
        .lines()
        .find_map(|line| line.strip_prefix("Tgid:"))
        .map(str::trim)
        .ok_or_else(|| ControlError::new("seccomp_socket_tgid", "missing Tgid"))?
        .parse::<u32>()
        .map_err(|error| ControlError::new("seccomp_socket_tgid", error.to_string()))?;
    Ok(Some(tgid))
}

pub(super) fn socket_symbol(syscall: u32) -> Result<&'static str, ControlError> {
    match syscall {
        SOCKET_PAYLOAD_SYSCALL_WRITE => Ok("write"),
        SOCKET_PAYLOAD_SYSCALL_SENDTO => Ok("sendto"),
        SOCKET_PAYLOAD_SYSCALL_WRITEV => Ok("writev"),
        SOCKET_PAYLOAD_SYSCALL_SENDMSG => Ok("sendmsg"),
        other => Err(ControlError::new(
            "seccomp_socket_symbol",
            format!("unsupported socket payload syscall {other}"),
        )),
    }
}

fn syscall_from_notification(
    notification: &libc::seccomp_notif,
) -> Result<Option<u32>, ControlError> {
    let raw = i64::from(notification.data.nr);
    if raw == libc::SYS_write {
        return Ok(Some(SOCKET_PAYLOAD_SYSCALL_WRITE));
    }
    if raw == libc::SYS_sendto {
        return Ok(Some(SOCKET_PAYLOAD_SYSCALL_SENDTO));
    }
    if raw == libc::SYS_writev {
        return Ok(Some(SOCKET_PAYLOAD_SYSCALL_WRITEV));
    }
    if raw == libc::SYS_sendmsg {
        return Ok(Some(SOCKET_PAYLOAD_SYSCALL_SENDMSG));
    }
    Ok(None)
}

fn key_requested_size(syscall: u32, notification: &libc::seccomp_notif) -> u64 {
    match syscall {
        SOCKET_PAYLOAD_SYSCALL_SENDMSG => 0,
        _ => notification.data.args[2],
    }
}
