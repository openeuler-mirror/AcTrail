//! Transport selection and client assembly for control calls.

use std::os::fd::BorrowedFd;

use control_contract::command::{ControlCommand, TrackAddCommand};
use control_contract::reply::{ControlError, ControlReply};
use uds_control_client::{RoundTripTransport, UdsControlClient};

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
