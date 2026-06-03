//! Target process launch control.

use std::ffi::{CString, OsString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use crate::{ToolError, ToolResult};

const EXEC_FAILURE_EXIT_CODE: i32 = 127;

pub(crate) struct PausedTarget {
    pid: u32,
}

impl PausedTarget {
    pub(crate) fn spawn(command: &[OsString]) -> ToolResult<Self> {
        let argv = argv(command)?;
        let raw_argv = raw_argv(&argv);
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(ToolError::new(format!(
                "fork target: {}",
                std::io::Error::last_os_error()
            )));
        }
        if pid == 0 {
            child_exec(&argv, &raw_argv);
        }
        let pid = u32::try_from(pid)
            .map_err(|error| ToolError::new(format!("target pid overflow: {error}")))?;
        wait_until_stopped(pid)?;
        Ok(Self { pid })
    }

    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn resume(&self) -> ToolResult<()> {
        let result = unsafe { libc::kill(self.pid as i32, libc::SIGCONT) };
        if result != 0 {
            return Err(ToolError::new(format!(
                "resume target pid={}: {}",
                self.pid,
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }

    pub(crate) fn try_wait(&mut self) -> ToolResult<Option<ExitStatus>> {
        let mut status = 0;
        let result = unsafe { libc::waitpid(self.pid as i32, &mut status, libc::WNOHANG) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Ok(None);
            }
            return Err(ToolError::new(format!(
                "wait target pid={}: {error}",
                self.pid
            )));
        }
        if result == 0 {
            return Ok(None);
        }
        Ok(Some(ExitStatus::from_raw(status)))
    }

    pub(crate) fn terminate(&mut self) {
        let _ = unsafe { libc::kill(self.pid as i32, libc::SIGTERM) };
    }
}

fn argv(command: &[OsString]) -> ToolResult<Vec<CString>> {
    if command.is_empty() {
        return Err(ToolError::new("probe command is empty"));
    }
    command
        .iter()
        .map(|value| {
            CString::new(value.as_os_str().as_bytes()).map_err(|_| {
                ToolError::new(format!(
                    "target argument contains interior NUL: {}",
                    value.to_string_lossy()
                ))
            })
        })
        .collect()
}

fn raw_argv(argv: &[CString]) -> Vec<*const libc::c_char> {
    let mut raw = argv
        .iter()
        .map(|value| value.as_ptr())
        .collect::<Vec<*const libc::c_char>>();
    raw.push(std::ptr::null());
    raw
}

fn child_exec(argv: &[CString], raw_argv: &[*const libc::c_char]) -> ! {
    unsafe {
        libc::raise(libc::SIGSTOP);
        libc::execvp(argv[0].as_ptr(), raw_argv.as_ptr());
        libc::_exit(EXEC_FAILURE_EXIT_CODE);
    }
}

fn wait_until_stopped(pid: u32) -> ToolResult<()> {
    let pid = i32::try_from(pid)
        .map_err(|error| ToolError::new(format!("target pid overflow: {error}")))?;
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(ToolError::new(format!(
                "wait target stop pid={pid}: {error}"
            )));
        }
        if libc::WIFSTOPPED(status) {
            return Ok(());
        }
        if libc::WIFEXITED(status) {
            return Err(ToolError::new(format!(
                "target pid={pid} exited before probe attach"
            )));
        }
        if libc::WIFSIGNALED(status) {
            return Err(ToolError::new(format!(
                "target pid={pid} was signaled before probe attach"
            )));
        }
    }
}
