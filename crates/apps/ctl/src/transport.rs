//! Transport selection and client assembly for control calls.

use control_contract::command::ControlCommand;
use control_contract::reply::{ControlError, ControlReply};
use uds_control_client::{RoundTripTransport, UdsControlClient};

pub trait ControlClientPort {
    fn send(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError>;
}

impl<T> ControlClientPort for UdsControlClient<T>
where
    T: RoundTripTransport,
{
    fn send(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError> {
        UdsControlClient::send(self, command)
    }
}
