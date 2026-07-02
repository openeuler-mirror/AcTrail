//! Controlled launch child lifecycle.

use std::ffi::{CString, OsString};
use std::io::Read;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

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
    pub(crate) fn probe_seccomp_notify_path(setup: &SeccompSetup) -> Result<(), String> {
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

    pub(super) fn close_listener_fd(&mut self) {
        self.listener_fd.take();
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
        // Forward SIGTERM/SIGINT to the agent while we block on it, so
        // `docker stop` / Ctrl-C reach the agent instead of being swallowed
        // by actrailctl. The agent is already exec'd and running here.
        install_signal_forwarding(self.pid);
        let result = wait_child_exit(self.pid);
        uninstall_signal_forwarding();
        result
    }

    pub(super) fn wait_with_monitor(
        mut self,
        mut keep_waiting: impl FnMut() -> Result<bool, String>,
        poll_interval: Duration,
    ) -> Result<i32, String> {
        loop {
            if let Some(status) = try_wait_child_exit(self.pid)? {
                return status;
            }
            let daemon_available = keep_waiting()
                .map_err(|error| format!("launch supervision check failed: {error}"))?;
            if !daemon_available {
                terminate_child(self.pid);
                self.env_writer.take();
                return Err("daemon became unavailable during launch; terminated child".to_string());
            }
            std::thread::sleep(poll_interval);
        }
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

/// Host pid of the agent child to forward signals to. `0` means "no child
/// installed" so the handler is a no-op outside of an active `wait`.
static FORWARD_CHILD_PID: AtomicI32 = AtomicI32::new(0);

/// Signal handler installed for SIGTERM/SIGINT while waiting on the agent.
///
/// MUST stay async-signal-safe: it only does an atomic load and `kill`. No
/// allocation, no `println!`, no locks, no panics.
extern "C" fn forward_signal(sig: libc::c_int) {
    let pid = FORWARD_CHILD_PID.load(Ordering::Relaxed);
    if pid > 0 {
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

fn set_forward_handler(handler: libc::sighandler_t) {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler;
        libc::sigemptyset(&mut action.sa_mask);
        // No SA_RESTART: let the blocking waitpid return EINTR so the
        // wait loop re-enters after the signal has been forwarded.
        action.sa_flags = 0;
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
    }
}

fn install_signal_forwarding(child_pid: libc::pid_t) {
    FORWARD_CHILD_PID.store(child_pid, Ordering::Relaxed);
    set_forward_handler(forward_signal as *const () as libc::sighandler_t);
}

fn uninstall_signal_forwarding() {
    set_forward_handler(libc::SIG_DFL);
    FORWARD_CHILD_PID.store(0, Ordering::Relaxed);
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
        if let Some(code) = exit_code_from_status(status) {
            return Ok(code);
        }
        // Neither exited nor signaled (e.g. stopped/continued): keep waiting.
    }
}

/// Map a terminated child's raw wait status to a process exit code.
///
/// Normal exit -> its own code. Killed by a signal -> `128 + signum`
/// (shell convention), so `docker stop` and Ctrl-C surface as an expected
/// termination rather than an error. `None` for a not-yet-terminated status.
fn exit_code_from_status(status: libc::c_int) -> Option<i32> {
    if libc::WIFEXITED(status) {
        Some(libc::WEXITSTATUS(status))
    } else if libc::WIFSIGNALED(status) {
        Some(128 + libc::WTERMSIG(status))
    } else {
        None
    }
}

fn try_wait_child_exit(child_pid: libc::pid_t) -> Result<Option<Result<i32, String>>, String> {
    let mut status = libc::c_int::default();
    loop {
        let result = unsafe { libc::waitpid(child_pid, &mut status, libc::WNOHANG) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("wait for launch child exit: {error}"));
        }
        if result == 0 {
            return Ok(None);
        }
        if libc::WIFEXITED(status) {
            let status = ExitStatus::from_raw(status);
            return Ok(Some(status.code().ok_or_else(|| {
                "launch child terminated without an exit code".to_string()
            })));
        }
        if libc::WIFSIGNALED(status) {
            return Ok(Some(Err(format!(
                "launch child terminated by signal {}",
                libc::WTERMSIG(status)
            ))));
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

#[cfg(test)]
mod tests {
    use super::{
        exit_code_from_status, install_signal_forwarding, uninstall_signal_forwarding,
        wait_child_exit,
    };
    use std::ffi::CString;
    use std::sync::Mutex;

    // Tests that touch the process-wide signal disposition + FORWARD_CHILD_PID
    // must not run concurrently with each other: one raises a process-wide
    // SIGTERM, and both mutate the global pid. Serialize just those.
    static SIGNAL_TEST_LOCK: Mutex<()> = Mutex::new(());

    // --- exit_code_from_status: deterministic, synthetic wait statuses. ---
    // The `assert!(libc::WIF…)` lines make these self-validating against the
    // platform's wait-status encoding: a wrong synthetic status fails loudly
    // rather than passing by accident.

    #[test]
    fn normal_exit_maps_to_its_code() {
        let status: libc::c_int = 7 << 8; // exited with code 7
        assert!(libc::WIFEXITED(status));
        assert_eq!(exit_code_from_status(status), Some(7));

        let status: libc::c_int = 0; // exited with code 0
        assert!(libc::WIFEXITED(status));
        assert_eq!(exit_code_from_status(status), Some(0));
    }

    #[test]
    fn signal_death_maps_to_128_plus_signum() {
        let status: libc::c_int = libc::SIGTERM; // killed by SIGTERM (15)
        assert!(libc::WIFSIGNALED(status));
        assert_eq!(exit_code_from_status(status), Some(128 + libc::SIGTERM));
        assert_eq!(exit_code_from_status(status), Some(143));

        let status: libc::c_int = libc::SIGKILL; // killed by SIGKILL (9)
        assert!(libc::WIFSIGNALED(status));
        assert_eq!(exit_code_from_status(status), Some(137));
    }

    // --- real-process tests: drive wait_child_exit + forwarding end-to-end. ---

    /// Fork + exec a child running `argv[0] .. argv[n]`; return its pid.
    fn spawn_exec_child(argv: &[&str]) -> libc::pid_t {
        let cstrings: Vec<CString> = argv
            .iter()
            .map(|a| CString::new(*a).expect("NUL-free argv"))
            .collect();
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
        if pid == 0 {
            // Child: exec the command directly. If exec fails, exit 127.
            let mut pointers: Vec<*const libc::c_char> =
                cstrings.iter().map(|c| c.as_ptr()).collect();
            pointers.push(std::ptr::null());
            unsafe {
                libc::execvp(cstrings[0].as_ptr(), pointers.as_mut_ptr());
                libc::_exit(127);
            }
        }
        pid
    }

    /// Normal exit: `/bin/true` → 0.
    #[test]
    fn wait_child_exit_normal_zero() {
        let pid = spawn_exec_child(&["/bin/true"]);
        let code = wait_child_exit(pid).expect("wait should succeed for /bin/true");
        assert_eq!(code, 0);
    }

    /// Normal exit with non-zero code: `sh -c 'exit 7'` → 7.
    #[test]
    fn wait_child_exit_normal_nonzero() {
        let pid = spawn_exec_child(&["sh", "-c", "exit 7"]);
        let code = wait_child_exit(pid).expect("wait should succeed for sh exit 7");
        assert_eq!(code, 7);
    }

    /// Signal exit: a child that kills itself with SIGTERM must map to
    /// `128 + 15 = 143` (shell convention), not be reported as an error.
    #[test]
    fn wait_child_exit_signal_maps_to_128_plus_signum() {
        let pid = spawn_exec_child(&["sh", "-c", "kill -TERM $$"]);
        let result = wait_child_exit(pid);
        assert_eq!(result, Ok(143), "WIFSIGNALED should map to 128+signum");
    }

    /// End-to-end forwarding: install the forwarder for a long-lived child,
    /// deliver SIGTERM to *this* process, and verify the handler forwards it to
    /// the child and `wait_child_exit` reaps it as 143. Exercises the
    /// async-signal-safe handler, the no-`SA_RESTART` install (waitpid must
    /// surface EINTR so the retry loop re-enters), and the 128+signum mapping.
    #[test]
    fn signal_forwarding_delivers_sigterm_to_child() {
        let _serialize = SIGNAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let child_pid = spawn_exec_child(&["sleep", "30"]);
        install_signal_forwarding(child_pid);

        // Deliver SIGTERM to ourselves; the handler forwards it to the child.
        unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };

        // waitpid gets EINTR (no SA_RESTART), loop re-enters, child reaped.
        let result = wait_child_exit(child_pid);
        uninstall_signal_forwarding();

        assert_eq!(result, Ok(143), "forwarded SIGTERM should reap child as 143");

        // No orphan: a second waitpid must fail (ECHILD) because it was reaped.
        let mut status = 0;
        let rc = unsafe { libc::waitpid(child_pid, &mut status, libc::WNOHANG) };
        assert!(rc < 0, "child should already be reaped (ECHILD), rc={rc}");
    }

    /// install/uninstall round-trip must clear the registered pid so a later
    /// signal is not forwarded to a stale (possibly reused) pid.
    #[test]
    fn forwarding_pid_cleared_after_uninstall() {
        use std::sync::atomic::Ordering;
        let _serialize = SIGNAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(super::FORWARD_CHILD_PID.load(Ordering::Relaxed), 0);
        install_signal_forwarding(999_999);
        assert_eq!(super::FORWARD_CHILD_PID.load(Ordering::Relaxed), 999_999);
        uninstall_signal_forwarding();
        assert_eq!(
            super::FORWARD_CHILD_PID.load(Ordering::Relaxed),
            0,
            "pid must be cleared after uninstall"
        );
    }
}
