//! Real Unix-socket serving loop for the daemon control plane.

use std::fs::{self, Permissions};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::Duration;

use config_core::daemon::{DEFAULT_CONTROL_PENDING_CONNECTION_MAX, SocketPermissions};
use uds_control_server::UdsControlConnection;

use crate::bootstrap::LocalDaemonServer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonRunError {
    pub stage: String,
    pub message: String,
}

impl DaemonRunError {
    fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }

    fn from_io(stage: &'static str, error: io::Error) -> Self {
        Self::new(stage, error.to_string())
    }
}

impl LocalDaemonServer {
    pub fn serve_forever(
        &mut self,
        socket_path: &Path,
        permissions: SocketPermissions,
    ) -> Result<(), DaemonRunError> {
        self.serve_forever_until(
            socket_path,
            permissions,
            DEFAULT_CONTROL_PENDING_CONNECTION_MAX,
            || false,
            || Ok(()),
        )
    }

    pub fn serve_forever_until<S, R>(
        &mut self,
        socket_path: &Path,
        permissions: SocketPermissions,
        pending_connection_max: u32,
        mut should_stop: S,
        on_ready: R,
    ) -> Result<(), DaemonRunError>
    where
        S: FnMut() -> bool,
        R: FnOnce() -> Result<(), DaemonRunError>,
    {
        let listener = bind_listener(socket_path, permissions)?;
        let pending_connection_max = usize::try_from(pending_connection_max).map_err(|error| {
            DaemonRunError::new(
                "control_pending_connection_max",
                format!("pending connection max overflow: {error}"),
            )
        })?;
        let mut control_connections = Vec::new();
        on_ready()?;
        while !should_stop() {
            self.serve_ready_cycle(&listener, &mut control_connections, pending_connection_max)?;
        }
        Ok(())
    }

    fn serve_ready_cycle(
        &mut self,
        listener: &UnixListener,
        control_connections: &mut Vec<UdsControlConnection>,
        pending_connection_max: usize,
    ) -> Result<(), DaemonRunError> {
        let control_fds = control_connections
            .iter()
            .map(UdsControlConnection::raw_fd)
            .collect::<Vec<_>>();
        let readiness = wait_for_ready(
            listener.as_raw_fd(),
            &control_fds,
            self.event_poll_fds()?,
            self.background_poll_timeout()
                .map_err(|error| DaemonRunError::new(error.code, error.message))?,
        )?;
        if readiness.event_source_ready || readiness.background_ready {
            self.drain_live_events()
                .map_err(|error| DaemonRunError::new(error.code, error.message))?;
        }
        if readiness.listener_ready {
            loop {
                match listener.accept() {
                    Ok((stream, _)) if control_connections.len() >= pending_connection_max => {
                        drop(stream);
                        tracing::warn!(
                            pending_connection_max,
                            "closing control connection because the pending connection limit is reached"
                        );
                    }
                    Ok((stream, _)) => {
                        stream.set_nonblocking(true).map_err(|error| {
                            DaemonRunError::from_io("control_nonblocking", error)
                        })?;
                        control_connections.push(UdsControlConnection::new(stream));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(DaemonRunError::from_io("accept", error)),
                }
            }
        }
        self.progress_control_connections(control_connections, &readiness.control_ready_fds);
        Ok(())
    }

    fn progress_control_connections(
        &mut self,
        control_connections: &mut Vec<UdsControlConnection>,
        ready_fds: &[RawFd],
    ) {
        control_connections.retain_mut(|connection| {
            if !ready_fds.contains(&connection.raw_fd()) {
                return true;
            }
            match self.progress_control_connection(connection) {
                Ok(done) => !done,
                Err(error) => {
                    tracing::warn!(error = %error, "closing failed control connection");
                    false
                }
            }
        });
    }

    fn event_poll_fds(&mut self) -> Result<Vec<RawFd>, DaemonRunError> {
        self.control_event_poll_fds()
            .map_err(|error| DaemonRunError::new(error.code, error.message))
    }
}

fn bind_listener(
    socket_path: &Path,
    permissions: SocketPermissions,
) -> Result<UnixListener, DaemonRunError> {
    let listener =
        UnixListener::bind(socket_path).map_err(|error| DaemonRunError::from_io("bind", error))?;
    if let Err(error) = listener.set_nonblocking(true) {
        return Err(setup_error_after_bind(
            socket_path,
            "set_nonblocking",
            error,
        ));
    }
    if let Err(error) = fs::set_permissions(socket_path, Permissions::from_mode(permissions.mode)) {
        return Err(setup_error_after_bind(
            socket_path,
            "set_permissions",
            error,
        ));
    }
    Ok(listener)
}

fn setup_error_after_bind(
    socket_path: &Path,
    stage: &'static str,
    error: io::Error,
) -> DaemonRunError {
    match fs::remove_file(socket_path) {
        Ok(()) => DaemonRunError::from_io(stage, error),
        Err(cleanup_error) => DaemonRunError::new(
            stage,
            format!(
                "{error}; cleanup {} failed: {cleanup_error}",
                socket_path.display()
            ),
        ),
    }
}

struct ReadySet {
    listener_ready: bool,
    control_ready_fds: Vec<RawFd>,
    event_source_ready: bool,
    background_ready: bool,
}

fn wait_for_ready(
    listener_fd: RawFd,
    control_fds: &[RawFd],
    event_fds: Vec<RawFd>,
    timeout: Option<Duration>,
) -> Result<ReadySet, DaemonRunError> {
    let mut fds = vec![poll_fd(listener_fd)];
    for control_fd in control_fds {
        fds.push(poll_fd(*control_fd));
    }
    for event_fd in event_fds {
        fds.push(poll_fd(event_fd));
    }
    let timeout_storage = timeout.map(duration_to_timespec).transpose()?;
    let timeout_ptr = timeout_storage
        .as_ref()
        .map(|value| value as *const libc::timespec)
        .unwrap_or(std::ptr::null());

    let ready = unsafe {
        libc::ppoll(
            fds.as_mut_ptr(),
            fds.len() as libc::nfds_t,
            timeout_ptr,
            std::ptr::null(),
        )
    };
    if ready < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(ReadySet {
                listener_ready: false,
                control_ready_fds: Vec::new(),
                event_source_ready: false,
                background_ready: false,
            });
        }
        return Err(DaemonRunError::from_io("ppoll", error));
    }
    if ready == 0 {
        return Ok(ReadySet {
            listener_ready: false,
            control_ready_fds: Vec::new(),
            event_source_ready: false,
            background_ready: timeout.is_some(),
        });
    }

    let listener_revents = fds[0].revents;
    ensure_valid_revents("listener", listener_revents)?;
    let control_end = 1 + control_fds.len();
    let mut control_ready_fds = Vec::new();
    for (control_fd, poll_fd) in control_fds.iter().zip(fds.iter().skip(1)) {
        if is_control_ready(poll_fd.revents) {
            control_ready_fds.push(*control_fd);
        }
    }
    let mut event_source_ready = false;
    for event_fd in fds.iter().skip(control_end) {
        ensure_valid_revents("event_source", event_fd.revents)?;
        event_source_ready |= is_readable(event_fd.revents);
    }

    Ok(ReadySet {
        listener_ready: is_readable(listener_revents),
        control_ready_fds,
        event_source_ready,
        background_ready: false,
    })
}

fn duration_to_timespec(duration: Duration) -> Result<libc::timespec, DaemonRunError> {
    Ok(libc::timespec {
        tv_sec: duration.as_secs().try_into().map_err(|error| {
            DaemonRunError::new(
                "poll_timeout",
                format!("duration seconds overflow: {error}"),
            )
        })?,
        tv_nsec: duration.subsec_nanos().into(),
    })
}

fn poll_fd(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

fn ensure_valid_revents(stage: &'static str, revents: i16) -> Result<(), DaemonRunError> {
    if revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err(DaemonRunError::new(
            format!("poll_{stage}"),
            format!("poll returned error flags {revents}"),
        ));
    }
    Ok(())
}

fn is_readable(revents: i16) -> bool {
    revents & (libc::POLLIN | libc::POLLHUP) != 0
}

fn is_control_ready(revents: i16) -> bool {
    revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
}
