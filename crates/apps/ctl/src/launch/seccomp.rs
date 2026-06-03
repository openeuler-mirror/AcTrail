//! Launch-time seccomp user-notify trampoline.

#[path = "seccomp/rules.rs"]
mod rules;

use std::ffi::{CString, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;

use config_core::daemon::{
    PayloadSocketSeccompSyscall, PayloadTlsSeccompSyscall, ProcessSeccompSyscall,
};
use control_contract::command::{ControlCommand, RegisterSeccompListenerCommand};
use control_contract::reply::ControlError;
use model_core::ids::{RequestId, TraceId};

use crate::transport::ControlClientPort;
use rules::{SeccompRule, append_rule, build_seccomp_rules};

pub(super) fn run_child_seccomp(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    trace_id: TraceId,
    argv: Vec<OsString>,
    payload_tls_syscalls: Vec<PayloadTlsSeccompSyscall>,
    payload_socket_syscalls: Vec<PayloadSocketSeccompSyscall>,
    payload_socket_max_segment_bytes: u32,
    process_syscalls: Vec<ProcessSeccompSyscall>,
    reserved_listener_fd: u32,
    envs: Vec<(std::ffi::OsString, std::ffi::OsString)>,
) -> Result<i32, String> {
    let argv = cstring_argv(argv)?;
    let envs = cstring_env(envs)?;
    let reserved_listener_fd = i32::try_from(reserved_listener_fd)
        .map_err(|error| format!("invalid seccomp_notify_reserved_listener_fd: {error}"))?;
    let rules = build_seccomp_rules(
        payload_tls_syscalls,
        payload_socket_syscalls,
        payload_socket_max_segment_bytes,
        process_syscalls,
    )?;
    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(format!(
            "fork launch child: {}",
            std::io::Error::last_os_error()
        ));
    }
    if child == 0 {
        child_exec_seccomp(&argv, &rules, reserved_listener_fd, &envs);
    }
    let child_pid = child as libc::pid_t;
    if let Err(error) = wait_child_stopped(child_pid) {
        terminate_child(child_pid);
        return Err(error);
    }
    let listener_fd = match duplicate_child_listener(child_pid, reserved_listener_fd) {
        Ok(fd) => fd,
        Err(error) => {
            terminate_child(child_pid);
            return Err(error);
        }
    };
    if let Err(error) = register_listener(client, request_id, trace_id, child_pid, &listener_fd) {
        terminate_child(child_pid);
        return Err(error);
    }
    drop(listener_fd);
    signal_child(child_pid, libc::SIGCONT)?;
    wait_child_exit(child_pid)
}

fn register_listener(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    trace_id: TraceId,
    child_pid: libc::pid_t,
    listener_fd: &OwnedFd,
) -> Result<(), String> {
    client
        .send(ControlCommand::RegisterSeccompListener(
            RegisterSeccompListenerCommand {
                request_id,
                trace_id,
                target_pid: child_pid as u32,
                listener_fd: Some(listener_fd.as_raw_fd()),
            },
        ))
        .map(|_| ())
        .map_err(format_control_error)
}

fn child_exec_seccomp(
    argv: &[CString],
    rules: &[SeccompRule],
    reserved_listener_fd: libc::c_int,
    envs: &[(CString, CString)],
) -> ! {
    match install_seccomp_listener(rules, reserved_listener_fd) {
        Ok(()) => unsafe {
            libc::raise(libc::SIGSTOP);
            for (key, value) in envs {
                if libc::setenv(key.as_ptr(), value.as_ptr(), 1) != 0 {
                    libc::_exit(libc::EXIT_FAILURE);
                }
            }
            let mut pointers = argv
                .iter()
                .map(|arg| arg.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect::<Vec<_>>();
            libc::execvp(argv[0].as_ptr(), pointers.as_mut_ptr());
            libc::_exit(libc::EXIT_FAILURE);
        },
        Err(error) => {
            eprintln!("{error}");
            unsafe { libc::_exit(libc::EXIT_FAILURE) }
        }
    }
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

fn wait_child_stopped(child_pid: libc::pid_t) -> Result<(), String> {
    let mut status = libc::c_int::default();
    loop {
        let result = unsafe { libc::waitpid(child_pid, &mut status, libc::WUNTRACED) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("wait for launch child stop: {error}"));
        }
        if libc::WIFSTOPPED(status) {
            return Ok(());
        }
        if libc::WIFEXITED(status) {
            return Err(format!(
                "launch child exited before exec with status {}",
                libc::WEXITSTATUS(status)
            ));
        }
        if libc::WIFSIGNALED(status) {
            return Err(format!(
                "launch child terminated before exec by signal {}",
                libc::WTERMSIG(status)
            ));
        }
    }
}

fn duplicate_child_listener(
    child_pid: libc::pid_t,
    reserved_listener_fd: libc::c_int,
) -> Result<OwnedFd, String> {
    let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, child_pid, 0) };
    if pidfd < 0 {
        return Err(format!(
            "pidfd_open launch child: {}",
            std::io::Error::last_os_error()
        ));
    }
    let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd as libc::c_int) };
    let listener_fd = unsafe {
        libc::syscall(
            libc::SYS_pidfd_getfd,
            pidfd.as_raw_fd(),
            reserved_listener_fd,
            0,
        )
    };
    if listener_fd < 0 {
        return Err(format!(
            "pidfd_getfd seccomp listener: {}",
            std::io::Error::last_os_error()
        ));
    }
    let listener = unsafe { OwnedFd::from_raw_fd(listener_fd as libc::c_int) };
    set_nonblocking(listener.as_raw_fd())?;
    Ok(listener)
}

fn set_nonblocking(fd: libc::c_int) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(format!(
            "read seccomp listener flags: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(format!(
            "set seccomp listener nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn wait_child_exit(child_pid: libc::pid_t) -> Result<i32, String> {
    let mut status = libc::c_int::default();
    loop {
        let result = unsafe { libc::waitpid(child_pid, &mut status, 0) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("wait for launch child exit: {error}"));
        }
        if libc::WIFEXITED(status) {
            return Ok(libc::WEXITSTATUS(status));
        }
        if libc::WIFSIGNALED(status) {
            return Err(format!(
                "launch child terminated by signal {}",
                libc::WTERMSIG(status)
            ));
        }
    }
}

fn signal_child(child_pid: libc::pid_t, signal: libc::c_int) -> Result<(), String> {
    let result = unsafe { libc::kill(child_pid, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "signal launch child: {}",
            std::io::Error::last_os_error()
        ))
    }
}

fn terminate_child(child_pid: libc::pid_t) {
    let _ = signal_child(child_pid, libc::SIGKILL);
    let mut status = libc::c_int::default();
    unsafe {
        libc::waitpid(child_pid, &mut status, 0);
    }
}

fn cstring_argv(argv: Vec<OsString>) -> Result<Vec<CString>, String> {
    if argv.is_empty() {
        return Err("launch requires a command after --".to_string());
    }
    argv.into_iter()
        .map(|arg| {
            CString::new(arg.as_bytes())
                .map_err(|error| format!("launch argument contains NUL: {error}"))
        })
        .collect()
}

fn cstring_env(envs: Vec<(OsString, OsString)>) -> Result<Vec<(CString, CString)>, String> {
    envs.into_iter()
        .map(|(key, value)| {
            let key = CString::new(key.as_bytes())
                .map_err(|error| format!("launch env key contains NUL: {error}"))?;
            let value = CString::new(value.as_bytes())
                .map_err(|error| format!("launch env value contains NUL: {error}"))?;
            Ok((key, value))
        })
        .collect()
}

fn format_control_error(error: ControlError) -> String {
    format!("control command failed: {}: {}", error.code, error.message)
}
