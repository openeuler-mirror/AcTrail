//! Controlled launch child lifecycle.

use std::ffi::{CString, OsString};
use std::io::Read;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use super::seccomp::SeccompSetup;

pub(super) enum ChildSetup {
    Plain,
    Seccomp(SeccompSetup),
}

pub(crate) struct ControlledChild {
    pid: libc::pid_t,
    env_writer: Option<OwnedFd>,
    listener_fd: Option<OwnedFd>,
}

impl ControlledChild {
    pub(crate) fn probe_seccomp_launch_path(setup: &SeccompSetup) -> Result<(), String> {
        let mut child = Self::spawn(
            vec![OsString::from("/bin/true")],
            ChildSetup::Seccomp(setup.clone()),
        )?;
        child.terminate();
        Ok(())
    }

    pub(super) fn spawn(argv: Vec<OsString>, setup: ChildSetup) -> Result<Self, String> {
        let argv = cstring_argv(argv)?;
        let uses_seccomp = matches!(setup, ChildSetup::Seccomp(_));
        let reserved_listener_fd = setup.reserved_listener_fd();
        let (read_fd, write_fd) = env_pipe()?;
        let child = unsafe { libc::fork() };
        if child < 0 {
            return Err(format!(
                "fork launch child: {}",
                std::io::Error::last_os_error()
            ));
        }
        if child == 0 {
            child_exec(argv, setup, read_fd, write_fd);
        }
        drop(read_fd);
        let mut child = Self {
            pid: child,
            env_writer: Some(write_fd),
            listener_fd: None,
        };
        if let Err(error) = wait_child_stopped(child.pid) {
            child.terminate();
            return Err(error);
        }
        if uses_seccomp {
            match duplicate_child_listener(child.pid, reserved_listener_fd) {
                Ok(listener_fd) => child.listener_fd = Some(listener_fd),
                Err(error) => {
                    child.terminate();
                    return Err(error);
                }
            }
        }
        Ok(child)
    }

    pub(super) fn pid(&self) -> u32 {
        self.pid as u32
    }

    pub(super) fn listener_fd(&self) -> Option<&OwnedFd> {
        self.listener_fd.as_ref()
    }

    pub(super) fn continue_with_envs(
        &mut self,
        envs: Vec<(OsString, OsString)>,
    ) -> Result<(), String> {
        let writer = self
            .env_writer
            .take()
            .ok_or_else(|| "launch child was already continued".to_string())?;
        signal_child(self.pid, libc::SIGCONT)?;
        write_env_payload(&writer, envs)?;
        drop(writer);
        Ok(())
    }

    pub(super) fn wait(self) -> Result<i32, String> {
        wait_child_exit(self.pid)
    }

    pub(super) fn terminate(&mut self) {
        terminate_child(self.pid);
        self.env_writer.take();
    }
}

impl ChildSetup {
    fn install_in_child(&self) -> Result<(), String> {
        match self {
            ChildSetup::Plain => Ok(()),
            ChildSetup::Seccomp(setup) => setup.install(),
        }
    }

    fn reserved_listener_fd(&self) -> libc::c_int {
        match self {
            ChildSetup::Plain => -1,
            ChildSetup::Seccomp(setup) => setup.reserved_listener_fd(),
        }
    }
}

fn env_pipe() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(format!(
            "create launch env pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe {
        (
            OwnedFd::from_raw_fd(fds[0] as libc::c_int),
            OwnedFd::from_raw_fd(fds[1] as libc::c_int),
        )
    })
}

fn child_exec(argv: Vec<CString>, setup: ChildSetup, read_fd: OwnedFd, write_fd: OwnedFd) -> ! {
    drop(write_fd);
    match setup.install_in_child() {
        Ok(()) => unsafe {
            libc::raise(libc::SIGSTOP);
        },
        Err(error) => {
            eprintln!("{error}");
            unsafe { libc::_exit(libc::EXIT_FAILURE) }
        }
    }
    match read_env_payload(&read_fd).and_then(set_envs) {
        Ok(()) => exec_argv(&argv),
        Err(error) => {
            eprintln!("{error}");
            unsafe { libc::_exit(libc::EXIT_FAILURE) }
        }
    }
}

fn exec_argv(argv: &[CString]) -> ! {
    unsafe {
        let mut pointers = argv
            .iter()
            .map(|arg| arg.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect::<Vec<_>>();
        libc::execvp(argv[0].as_ptr(), pointers.as_mut_ptr());
        eprintln!(
            "launch child exec {}: {}",
            argv[0].to_string_lossy(),
            std::io::Error::last_os_error()
        );
        libc::_exit(libc::EXIT_FAILURE);
    }
}

fn write_env_payload(writer: &OwnedFd, envs: Vec<(OsString, OsString)>) -> Result<(), String> {
    let mut file = std::fs::File::from(
        writer
            .try_clone()
            .map_err(|error| format!("clone launch env pipe writer: {error}"))?,
    );
    for (key, value) in envs {
        let key = CString::new(key.as_os_str().as_bytes())
            .map_err(|error| format!("launch env key contains NUL: {error}"))?;
        let value = CString::new(value.as_os_str().as_bytes())
            .map_err(|error| format!("launch env value contains NUL: {error}"))?;
        file.write_all(key.as_bytes_with_nul())
            .map_err(|error| format!("write launch env key: {error}"))?;
        file.write_all(value.as_bytes_with_nul())
            .map_err(|error| format!("write launch env value: {error}"))?;
    }
    Ok(())
}

fn read_env_payload(read_fd: &OwnedFd) -> Result<Vec<(CString, CString)>, String> {
    let mut bytes = Vec::new();
    let mut file = std::fs::File::from(
        read_fd
            .try_clone()
            .map_err(|error| format!("clone launch env pipe reader: {error}"))?,
    );
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("read launch env payload: {error}"))?;
    parse_env_payload(&bytes)
}

fn parse_env_payload(bytes: &[u8]) -> Result<Vec<(CString, CString)>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
    if parts.last().is_some_and(|part| part.is_empty()) {
        parts.pop();
    }
    if parts.len() % 2 != 0 {
        return Err("launch env payload has an unpaired key/value".to_string());
    }
    parts
        .chunks_exact(2)
        .map(|pair| {
            Ok((
                CString::new(pair[0])
                    .map_err(|error| format!("launch env key contains NUL: {error}"))?,
                CString::new(pair[1])
                    .map_err(|error| format!("launch env value contains NUL: {error}"))?,
            ))
        })
        .collect()
}

fn set_envs(envs: Vec<(CString, CString)>) -> Result<(), String> {
    for (key, value) in envs {
        if unsafe { libc::setenv(key.as_ptr(), value.as_ptr(), 1) } != 0 {
            return Err(format!(
                "set launch env {}: {}",
                key.to_string_lossy(),
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

fn cstring_argv(argv: Vec<OsString>) -> Result<Vec<CString>, String> {
    if argv.is_empty() {
        return Err("launch requires a command after --".to_string());
    }
    argv.into_iter()
        .map(|arg| {
            CString::new(arg.as_os_str().as_bytes())
                .map_err(|error| format!("launch argument contains NUL: {error}"))
        })
        .collect()
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
            "mark seccomp listener nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn signal_child(child_pid: libc::pid_t, signal: libc::c_int) -> Result<(), String> {
    if unsafe { libc::kill(child_pid, signal) } != 0 {
        return Err(format!(
            "signal launch child: {}",
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
            let status = ExitStatus::from_raw(status);
            return status
                .code()
                .ok_or_else(|| "launch child terminated without an exit code".to_string());
        }
        if libc::WIFSIGNALED(status) {
            return Err(format!(
                "launch child terminated by signal {}",
                libc::WTERMSIG(status)
            ));
        }
    }
}

fn terminate_child(child_pid: libc::pid_t) {
    let _ = unsafe { libc::kill(child_pid, libc::SIGKILL) };
    let mut status = libc::c_int::default();
    loop {
        let result = unsafe { libc::waitpid(child_pid, &mut status, 0) };
        if result >= 0 {
            break;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            break;
        }
    }
}
