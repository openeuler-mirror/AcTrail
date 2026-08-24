//! Poll-loop owner for listener admission, connection deadlines, and async dispatch results.

use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Instant;

use sandbox_control::{
    SandboxControlRejection, SandboxControlRejectionCode, SandboxControlResponse,
};

use crate::connection::SandboxControlConnection;
use crate::dispatcher::{DispatchAdmission, Dispatcher};
use crate::server::BoundSocket;
use crate::{
    SandboxControlCodec, SandboxControlConnectionLimits, SandboxControlUdsError,
    SandboxControlUdsStage,
};

pub(crate) struct ServerRuntime {
    listener: UnixListener,
    _socket_owner: BoundSocket,
    stop_reader: UnixStream,
    dispatcher: Dispatcher,
    connections: Vec<SandboxControlConnection>,
    accepted_connection_max: usize,
    limits: SandboxControlConnectionLimits,
    codec: SandboxControlCodec,
    next_connection_id: u64,
    poll_fds: Vec<libc::pollfd>,
}

impl ServerRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        listener: UnixListener,
        socket_owner: BoundSocket,
        stop_reader: UnixStream,
        dispatcher: Dispatcher,
        accepted_connection_max: usize,
        limits: SandboxControlConnectionLimits,
        codec: SandboxControlCodec,
    ) -> Self {
        Self {
            listener,
            _socket_owner: socket_owner,
            stop_reader,
            dispatcher,
            connections: Vec::new(),
            accepted_connection_max,
            limits,
            codec,
            next_connection_id: 0,
            poll_fds: Vec::new(),
        }
    }

    pub(crate) fn run(mut self) -> Result<(), SandboxControlUdsError> {
        loop {
            self.prepare_poll_fds();
            let timeout_ms = self.poll_timeout_ms();
            let ready = unsafe {
                libc::poll(
                    self.poll_fds.as_mut_ptr(),
                    self.poll_fds.len() as libc::nfds_t,
                    timeout_ms,
                )
            };
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(io_error(SandboxControlUdsStage::Accept, error));
            }
            if has_terminal_event(self.poll_fds[1].revents) {
                return Ok(());
            }
            let mut topology_changed = false;
            if self.poll_fds[2].revents != 0 {
                topology_changed = self.apply_dispatch_results();
                if self.dispatcher.is_finished() {
                    return Err(SandboxControlUdsError::new(
                        SandboxControlUdsStage::Dispatch,
                        "sandbox control dispatcher stopped",
                    ));
                }
            }
            if topology_changed {
                continue;
            }
            self.progress_connections();
            if self.poll_fds[0].revents & libc::POLLIN != 0 {
                self.accept_budgeted()?;
            }
        }
    }

    fn prepare_poll_fds(&mut self) {
        self.poll_fds.clear();
        let listener_events = if self.connections.len() < self.accepted_connection_max {
            libc::POLLIN
        } else {
            0
        };
        self.poll_fds
            .push(poll_fd(self.listener.as_raw_fd(), listener_events));
        self.poll_fds
            .push(poll_fd(self.stop_reader.as_raw_fd(), libc::POLLIN));
        self.poll_fds
            .push(poll_fd(self.dispatcher.wake_raw_fd(), libc::POLLIN));
        self.poll_fds.extend(
            self.connections
                .iter()
                .map(|connection| poll_fd(connection.raw_fd(), connection.poll_events())),
        );
    }

    fn progress_connections(&mut self) {
        let now = Instant::now();
        for index in (0..self.connections.len()).rev() {
            if self.connections[index].expired(now) {
                self.connections.swap_remove(index);
                continue;
            }
            let revents = self.poll_fds[index + 3].revents;
            if revents == 0 {
                continue;
            }
            let result = self.progress_connection(index);
            if result.unwrap_or(true) {
                self.connections.swap_remove(index);
            }
        }
    }

    fn progress_connection(&mut self, index: usize) -> Result<bool, SandboxControlUdsError> {
        if let Some(command) = self.connections[index].read_command(&self.codec, self.limits)? {
            let id = self.connections[index].id();
            match self.dispatcher.admit(id, command) {
                DispatchAdmission::Accepted => {}
                DispatchAdmission::Busy => {
                    let response = rejection(
                        SandboxControlRejectionCode::Busy,
                        "sandbox connection control is busy",
                    );
                    self.connections[index].set_response(&self.codec, self.limits, &response)?;
                }
                DispatchAdmission::Closed => {
                    let response = rejection(
                        SandboxControlRejectionCode::ShuttingDown,
                        "sandbox control dispatcher is unavailable",
                    );
                    self.connections[index].set_response(&self.codec, self.limits, &response)?;
                }
            }
        }
        self.connections[index].write_response()
    }

    fn apply_dispatch_results(&mut self) -> bool {
        let now = Instant::now();
        let mut topology_changed = false;
        for result in self.dispatcher.drain_results() {
            let Some(index) = self
                .connections
                .iter()
                .position(|connection| connection.id() == result.connection_id)
            else {
                continue;
            };
            if self.connections[index].expired(now)
                || self.connections[index]
                    .set_response(&self.codec, self.limits, &result.response)
                    .is_err()
            {
                self.connections.swap_remove(index);
                topology_changed = true;
            }
        }
        topology_changed
    }

    fn accept_budgeted(&mut self) -> Result<(), SandboxControlUdsError> {
        let budget = self
            .accepted_connection_max
            .saturating_sub(self.connections.len());
        for _ in 0..budget {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let id = self.advance_connection_id();
                    if let Ok(connection) = SandboxControlConnection::new(id, stream, self.limits) {
                        self.connections.push(connection);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionAborted => continue,
                Err(error) => return Err(io_error(SandboxControlUdsStage::Accept, error)),
            }
        }
        Ok(())
    }

    fn advance_connection_id(&mut self) -> u64 {
        self.next_connection_id = self.next_connection_id.wrapping_add(1).max(1);
        self.next_connection_id
    }

    fn poll_timeout_ms(&self) -> i32 {
        let Some(deadline) = self
            .connections
            .iter()
            .map(|connection| connection.deadline())
            .min()
        else {
            return -1;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        i32::try_from(remaining.as_millis().max(1)).unwrap_or(i32::MAX)
    }
}

fn rejection(code: SandboxControlRejectionCode, reason: &'static str) -> SandboxControlResponse {
    SandboxControlResponse::Rejected(
        SandboxControlRejection::new(code, reason).expect("fixed rejection reason is valid"),
    )
}

fn has_terminal_event(revents: i16) -> bool {
    revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0
}

fn poll_fd(fd: RawFd, events: i16) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

fn io_error(stage: SandboxControlUdsStage, error: std::io::Error) -> SandboxControlUdsError {
    SandboxControlUdsError::new(stage, error.to_string())
}
