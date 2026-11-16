//! Hands save, restore, force, resume, delete and disconnect to the daemon, which authorizes them.

use std::time::{SystemTime, UNIX_EPOCH};

use control_contract::checkpoint::{CheckpointAction, CheckpointCommand, CheckpointReply};
use control_contract::command::ControlCommand;
use control_contract::reply::ControlReply;
use model_core::ids::RequestId;
use serde_json::json;
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use super::{Failure, Request};
use crate::http::Response;

pub(super) fn send(request: &Request<'_>, command: &str) -> Result<Response, Failure> {
    let trace_id = request.trace_id;
    let action = match command {
        "save" => CheckpointAction::Save { trace_id },
        "restore" => CheckpointAction::Restore {
            trace_id,
            checkpoint_id: request.id("checkpoint")?,
        },
        "resume" => CheckpointAction::Resume { trace_id },
        "delete" => CheckpointAction::Delete {
            trace_id,
            checkpoint_id: request.id("checkpoint")?,
        },
        "disconnect" => CheckpointAction::Disconnect { trace_id },
        // Operations of other Traces are refused here, so the URL names the Trace acted on.
        "force" => CheckpointAction::Force {
            operation_id: request.operation(&request.reader()?)?.operation_id.get(),
        },
        _ => unreachable!("routed only for checkpoint commands"),
    };
    let reply = UdsControlClient::new(UdsSocketTransport::new(request.socket_path.to_path_buf()))
        .send(ControlCommand::Checkpoint(CheckpointCommand {
            request_id: request_id()?,
            action,
        }))
        .map_err(|error| {
            Failure::BadRequest(format!(
                "daemon checkpoint {command} failed: {}: {}",
                error.code, error.message
            ))
        })?;
    let body = match reply {
        ControlReply::Checkpoint(CheckpointReply::OperationAccepted { operation_id }) => {
            json!({ "accepted": true, "operation_id": operation_id })
        }
        ControlReply::Checkpoint(CheckpointReply::Accepted) => json!({ "accepted": true }),
        _ => {
            return Err(Failure::Internal(format!(
                "daemon returned an unexpected reply for checkpoint {command}"
            )));
        }
    };
    Ok(Response::json(body.to_string()))
}

pub(super) fn request_id() -> Result<RequestId, Failure> {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_micros()).ok())
        .ok_or_else(|| Failure::Internal("the system clock cannot form a request id".into()))?;
    Ok(RequestId::new(micros))
}
