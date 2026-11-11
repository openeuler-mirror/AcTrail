//! The checkpoint history and operations of a Trace, with the LLM call each boundary matched.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use checkpoint_core::model::{
    Boundary, CheckpointRecord, OperationRecord, Release, ScopeConfig, TraceRecord,
};
use control_contract::checkpoint::{CheckpointAction, CheckpointCommand, CheckpointReply};
use control_contract::command::ControlCommand;
use control_contract::reply::ControlReply;
use serde_json::{Value, json};
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use super::{Failure, Request, actions, catalog_error, navigation};
use crate::http::Response;

pub(super) fn render(request: &Request<'_>) -> Result<Response, Failure> {
    let history = request
        .reader()?
        .history(request.trace_id)
        .map_err(catalog_error)?;
    let llm_calls = navigation::llm_calls(request, &history.operations);
    let runtime = history.trace.as_ref().and_then(|_| runtime_status(request));
    let body = json!({
        "enabled": true,
        "poll_interval_ms": request.config.poll_interval_ms,
        "trace": history.trace.as_ref().map(trace_json),
        "runtime": runtime,
        "checkpoints": history.checkpoints.iter().map(checkpoint_json).collect::<Vec<_>>(),
        "operations": history.operations
            .iter()
            .map(|operation| operation_json(operation, llm_calls.get(&operation.operation_id)))
            .collect::<Vec<_>>(),
    });
    Ok(Response::json(body.to_string()))
}

/// History stays available when the live daemon cannot answer; its pause state is then unknown.
fn runtime_status(request: &Request<'_>) -> Option<Value> {
    let transport = UdsSocketTransport::new(request.socket_path)
        .with_timeout(Duration::from_millis(request.config.status_timeout_ms));
    let reply = UdsControlClient::new(transport)
        .send(ControlCommand::Checkpoint(CheckpointCommand {
            request_id: actions::request_id().ok()?,
            action: CheckpointAction::Status {
                trace_id: request.trace_id,
            },
        }))
        .ok()?;
    match reply {
        ControlReply::Checkpoint(CheckpointReply::Status { held, busy }) => {
            Some(json!({ "held": held, "busy": busy }))
        }
        _ => None,
    }
}

fn trace_json(trace: &TraceRecord) -> Value {
    json!({
        "created_at": millis(trace.created_at),
        "ended_at": trace.ended_at.map(millis),
        "disconnected_at": trace.disconnected.as_ref().map(|disconnected| millis(disconnected.disconnected_at)),
        "disconnect_retryable": trace.disconnected.as_ref().is_some_and(|disconnected| disconnected.can_retry()),
        "scopes": trace.scopes.iter().map(scope_json).collect::<Vec<_>>(),
    })
}

fn scope_json(scope: &ScopeConfig) -> Value {
    json!({
        "id": scope.id,
        "root": scope.root,
        "excludes": scope.excludes,
        "backend_instance": scope.backend_instance,
    })
}

fn checkpoint_json(checkpoint: &CheckpointRecord) -> Value {
    let deletion = checkpoint.deletion.as_ref();
    json!({
        "checkpoint_id": checkpoint.checkpoint_id.get(),
        "operation_id": checkpoint.operation_id.get(),
        "created_at": millis(checkpoint.created_at),
        "restorable": checkpoint.restorable(),
        "invalidated_by": checkpoint.invalidated_by.map(|operation| operation.get()),
        "deleted_at": deletion.map(|deletion| millis(deletion.deleted_at)),
        "release": deletion.map(|deletion| match &deletion.release {
            Release::Pending => "pending",
            Release::Released => "released",
            Release::Failed(_) => "failed",
        }),
    })
}

fn operation_json(operation: &OperationRecord, llm_call: Option<&String>) -> Value {
    json!({
        "operation_id": operation.operation_id.get(),
        "kind": operation.kind.as_str(),
        "trigger": operation.trigger.as_str(),
        "phase": operation.phase.as_str(),
        "outcome": operation.outcome.map(|outcome| outcome.as_str()),
        "requested_at": millis(operation.requested_at),
        "started_at": operation.started_at.map(millis),
        "finished_at": operation.finished_at.map(millis),
        "target_checkpoint": operation.target_checkpoint.map(|checkpoint| checkpoint.get()),
        "created_checkpoint": operation.created_checkpoint.map(|checkpoint| checkpoint.get()),
        "boundary": operation.boundary.as_ref().map(boundary_json),
        "llm_call_action_id": llm_call,
        "failure": operation.failure,
    })
}

fn boundary_json(boundary: &Boundary) -> Value {
    json!({
        "observed_at": millis(boundary.observed_at),
        "request_action_id": boundary.request_action_id,
    })
}

fn millis(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
