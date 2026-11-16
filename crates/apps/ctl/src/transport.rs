//! Transport selection and client assembly for control calls.

use std::os::fd::{BorrowedFd, RawFd};
use std::path::PathBuf;

use control_contract::command::{ControlCommand, TrackAddCommand};
use control_contract::reply::{ControlError, ControlReply};
use uds_control_client::{RoundTripTransport, UdsControlClient, UdsSocketTransport};

pub(super) struct OperatorTransport {
    inner: UdsSocketTransport,
}

impl OperatorTransport {
    pub(super) fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            inner: UdsSocketTransport::new(socket_path),
        }
    }

    fn failure(&self, error: String) -> String {
        format!(
            "control request via socket {} failed: {error}; check that the daemon is running, verify the configured socket path and socket permissions",
            self.inner.socket_path().display()
        )
    }
}

impl RoundTripTransport for OperatorTransport {
    fn send(&mut self, request: Vec<u8>) -> Result<Vec<u8>, String> {
        self.inner
            .send(request)
            .map_err(|error| self.failure(error))
    }

    fn send_with_fds(&mut self, request: Vec<u8>, fds: &[RawFd]) -> Result<Vec<u8>, String> {
        self.inner
            .send_with_fds(request, fds)
            .map_err(|error| self.failure(error))
    }
}

pub trait ControlClientPort {
    fn send(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError>;

    fn send_launch_track_add(
        &mut self,
        command: TrackAddCommand,
        pidfd: BorrowedFd<'_>,
    ) -> Result<ControlReply, ControlError>;
}

impl<T> ControlClientPort for UdsControlClient<T>
where
    T: RoundTripTransport,
{
    fn send(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError> {
        UdsControlClient::send(self, command)
    }

    fn send_launch_track_add(
        &mut self,
        command: TrackAddCommand,
        pidfd: BorrowedFd<'_>,
    ) -> Result<ControlReply, ControlError> {
        UdsControlClient::send_launch_track_add(self, command, pidfd)
    }
}
