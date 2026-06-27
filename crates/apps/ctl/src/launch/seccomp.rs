//! Launch-time seccomp user-notify setup.

#[path = "seccomp/rules.rs"]
mod rules;

use std::os::fd::{AsRawFd, OwnedFd};

use config_core::daemon::{
    NetworkControlSeccompSyscall, PayloadSocketSeccompSyscall, PayloadTlsSeccompSyscall,
    ProcessSeccompSyscall,
};
use control_contract::command::{ControlCommand, ProcessRef, RegisterSeccompListenerCommand};
use control_contract::reply::ControlError;
use model_core::ids::{RequestId, TraceId};

use crate::transport::ControlClientPort;
use rules::{SeccompRule, append_rule, build_seccomp_rules};

pub(crate) struct SeccompSetup {
    rules: Vec<SeccompRule>,
    reserved_listener_fd: libc::c_int,
}

impl Clone for SeccompSetup {
    fn clone(&self) -> Self {
        Self {
            rules: self.rules.clone(),
            reserved_listener_fd: self.reserved_listener_fd,
        }
    }
}

impl SeccompSetup {
    pub(crate) fn new(
        payload_tls_syscalls: Vec<PayloadTlsSeccompSyscall>,
        payload_socket_syscalls: Vec<PayloadSocketSeccompSyscall>,
        payload_socket_max_segment_bytes: u32,
        process_syscalls: Vec<ProcessSeccompSyscall>,
        network_syscalls: Vec<NetworkControlSeccompSyscall>,
        reserved_listener_fd: u32,
    ) -> Result<Self, String> {
        let reserved_listener_fd = i32::try_from(reserved_listener_fd)
            .map_err(|error| format!("invalid seccomp_notify_reserved_listener_fd: {error}"))?;
        let rules = build_seccomp_rules(
            payload_tls_syscalls,
            payload_socket_syscalls,
            payload_socket_max_segment_bytes,
            process_syscalls,
            network_syscalls,
        )?;
        Ok(Self {
            rules,
            reserved_listener_fd,
        })
    }

    pub(super) fn reserved_listener_fd(&self) -> libc::c_int {
        self.reserved_listener_fd
    }

    pub(super) fn install(&self) -> Result<(), String> {
        install_seccomp_listener(&self.rules, self.reserved_listener_fd)
    }
}

pub(super) fn register_listener(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    trace_id: TraceId,
    child: ProcessRef,
    listener_fd: &OwnedFd,
) -> Result<(), String> {
    client
        .send(ControlCommand::RegisterSeccompListener(
            RegisterSeccompListenerCommand {
                request_id,
                trace_id,
                target: child,
                listener_fd: Some(listener_fd.as_raw_fd()),
            },
        ))
        .map(|_| ())
        .map_err(format_control_error)
}

fn install_seccomp_listener(
    rules: &[SeccompRule],
    reserved_listener_fd: libc::c_int,
) -> Result<(), String> {
    if rules.is_empty() {
        return Err("seccomp syscall filter must contain at least one syscall".to_string());
    }
    let prctl_result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if prctl_result != 0 {
        return Err(format!(
            "set no_new_privs before seccomp: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut filter = Vec::new();
    for rule in rules {
        append_rule(&mut filter, *rule);
    }
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_RET | libc::BPF_K) as u16,
            libc::SECCOMP_RET_ALLOW,
        )
    });
    let mut program = libc::sock_fprog {
        len: filter
            .len()
            .try_into()
            .map_err(|error| format!("seccomp filter length overflow: {error}"))?,
        filter: filter.as_mut_ptr(),
    };
    let listener_fd = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            libc::SECCOMP_FILTER_FLAG_NEW_LISTENER,
            &mut program,
        )
    };
    if listener_fd < 0 {
        return Err(format!(
            "install seccomp user notification filter: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::dup2(listener_fd as libc::c_int, reserved_listener_fd) } < 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::close(listener_fd as libc::c_int);
        }
        return Err(format!("dup seccomp listener fd: {error}"));
    }
    let flags = unsafe { libc::fcntl(reserved_listener_fd, libc::F_GETFD) };
    if flags < 0
        || unsafe {
            libc::fcntl(
                reserved_listener_fd,
                libc::F_SETFD,
                flags | libc::FD_CLOEXEC,
            )
        } < 0
    {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::close(listener_fd as libc::c_int);
        }
        return Err(format!("mark seccomp listener close-on-exec: {error}"));
    }
    if listener_fd as libc::c_int != reserved_listener_fd {
        unsafe {
            libc::close(listener_fd as libc::c_int);
        }
    }
    Ok(())
}

fn format_control_error(error: ControlError) -> String {
    format!("control command failed: {}: {}", error.code, error.message)
}
