//! Dispatch from validated input to control-plane contracts.

use control_contract::command::{
    ControlCommand, DoctorCommand, ListTracesCommand, TrackAddCommand, TrackRemoveCommand,
};
use control_contract::reply::{ControlError, ControlReply};
use model_core::ids::RequestId;

use crate::args::CtlCommand;
use crate::transport::ControlClientPort;

pub fn dispatch(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    command: CtlCommand,
) -> Result<ControlReply, ControlError> {
    let control_command = match command {
        CtlCommand::TrackAdd {
            root_pid,
            display_name,
            profile_name,
            tags,
        } => ControlCommand::TrackAdd(TrackAddCommand {
            request_id,
            root_pid,
            display_name,
            profile_name,
            tags,
            launch_mode: false,
            initial_suppressed_fds: Vec::new(),
        }),
        CtlCommand::TrackRemove { selector } => ControlCommand::TrackRemove(TrackRemoveCommand {
            request_id,
            selector,
        }),
        CtlCommand::ListTraces { selector } => ControlCommand::ListTraces(ListTracesCommand {
            request_id,
            selector,
        }),
        CtlCommand::Doctor => ControlCommand::Doctor(DoctorCommand { request_id }),
        CtlCommand::Init { .. } => {
            return Err(ControlError::new(
                "invalid_dispatch",
                "init is handled by the local actrailctl process",
            ));
        }
        CtlCommand::Launch { .. } => {
            return Err(ControlError::new(
                "invalid_dispatch",
                "launch is handled by the local actrailctl process",
            ));
        }
        CtlCommand::Clean { .. } => {
            return Err(ControlError::new(
                "invalid_dispatch",
                "clean is handled by the local actrailctl process",
            ));
        }
    };

    client.send(control_command)
}
