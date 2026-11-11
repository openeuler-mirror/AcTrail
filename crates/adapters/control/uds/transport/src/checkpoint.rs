//! Checkpoint command, reply and launch configuration fields.
//!
//! Commands keep one field per value. History replies carry their records as one JSON field.

use control_contract::checkpoint::{
    CheckpointAction, CheckpointCommand, CheckpointFilterMode, CheckpointReply,
    LaunchCheckpointConfig, LaunchCheckpointScope,
};
use control_contract::command::ControlCommand;
use model_core::ids::{RequestId, TraceId};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::optional::{optional_u64, parse_optional_u64};
use super::{ControlCodecError, field, parse_bool, parse_u32, parse_u64, parse_usize};

pub(super) const COMMAND_OPCODE: &str = "checkpoint_v2";
pub(super) const REPLY_OPCODE: &str = "reply_checkpoint_v1";

pub(super) fn encode_command(fields: &mut Vec<String>, command: &CheckpointCommand) {
    fields.push(COMMAND_OPCODE.to_string());
    fields.push(command.request_id.get().to_string());
    let (action, values): (&str, Vec<String>) = match command.action {
        CheckpointAction::Save { trace_id } => ("save", vec![trace_id.get().to_string()]),
        CheckpointAction::Restore {
            trace_id,
            checkpoint_id,
        } => (
            "restore",
            vec![trace_id.get().to_string(), checkpoint_id.to_string()],
        ),
        CheckpointAction::Delete {
            trace_id,
            checkpoint_id,
        } => (
            "delete",
            vec![trace_id.get().to_string(), checkpoint_id.to_string()],
        ),
        CheckpointAction::Disconnect { trace_id } => {
            ("disconnect", vec![trace_id.get().to_string()])
        }
        CheckpointAction::Force { operation_id } => ("force", vec![operation_id.to_string()]),
        CheckpointAction::Resume { trace_id } => ("resume", vec![trace_id.get().to_string()]),
        CheckpointAction::List {
            trace_id,
            filter_mode,
        } => {
            let mut values = vec![trace_id.get().to_string()];
            encode_filter_mode(&mut values, filter_mode);
            ("list", values)
        }
        CheckpointAction::Operations { trace_id } => {
            ("operations", vec![trace_id.get().to_string()])
        }
        CheckpointAction::Operation { operation_id } => {
            ("operation", vec![operation_id.to_string()])
        }
        CheckpointAction::Status { trace_id } => ("status", vec![trace_id.get().to_string()]),
    };
    fields.push(action.to_string());
    fields.extend(values);
}

pub(super) fn decode_command(fields: &[String]) -> Result<ControlCommand, ControlCodecError> {
    let request_id = RequestId::new(parse_u64(field(fields, 1)?, "request_id")?);
    let id = |index: usize, name: &str| parse_u64(field(fields, 3 + index)?, name);
    let trace = |index: usize| id(index, "trace_id").map(TraceId::new);
    let action = match field(fields, 2)?.as_str() {
        "save" => CheckpointAction::Save {
            trace_id: trace(0)?,
        },
        "restore" => CheckpointAction::Restore {
            trace_id: trace(0)?,
            checkpoint_id: id(1, "checkpoint_id")?,
        },
        "delete" => CheckpointAction::Delete {
            trace_id: trace(0)?,
            checkpoint_id: id(1, "checkpoint_id")?,
        },
        "disconnect" => CheckpointAction::Disconnect {
            trace_id: trace(0)?,
        },
        "force" => CheckpointAction::Force {
            operation_id: id(0, "operation_id")?,
        },
        "resume" => CheckpointAction::Resume {
            trace_id: trace(0)?,
        },
        "list" => CheckpointAction::List {
            trace_id: trace(0)?,
            filter_mode: decode_filter_mode(fields, 4)?,
        },
        "operations" => CheckpointAction::Operations {
            trace_id: trace(0)?,
        },
        "operation" => CheckpointAction::Operation {
            operation_id: id(0, "operation_id")?,
        },
        "status" => CheckpointAction::Status {
            trace_id: trace(0)?,
        },
        _ => {
            return Err(ControlCodecError::new(
                "decode",
                "unknown checkpoint action",
            ));
        }
    };
    Ok(ControlCommand::Checkpoint(CheckpointCommand {
        request_id,
        action,
    }))
}

/// The mode name, followed by the values that mode takes.
fn encode_filter_mode(values: &mut Vec<String>, filter_mode: CheckpointFilterMode) {
    match filter_mode {
        CheckpointFilterMode::All => values.push("all".to_string()),
        CheckpointFilterMode::Last(count) => {
            values.push("last".to_string());
            values.push(count.to_string());
        }
        CheckpointFilterMode::Created {
            since_unix_nanos,
            until_unix_nanos,
        } => {
            values.push("created".to_string());
            values.push(optional_u64(since_unix_nanos));
            values.push(optional_u64(until_unix_nanos));
        }
    }
}

fn decode_filter_mode(
    fields: &[String],
    index: usize,
) -> Result<CheckpointFilterMode, ControlCodecError> {
    match field(fields, index)?.as_str() {
        "all" => Ok(CheckpointFilterMode::All),
        "last" => parse_u32(field(fields, index + 1)?, "count").map(CheckpointFilterMode::Last),
        "created" => Ok(CheckpointFilterMode::Created {
            since_unix_nanos: parse_optional_u64(field(fields, index + 1)?, "since")?,
            until_unix_nanos: parse_optional_u64(field(fields, index + 2)?, "until")?,
        }),
        _ => Err(ControlCodecError::new(
            "decode",
            "unknown checkpoint filter mode",
        )),
    }
}

pub(super) fn encode_reply(fields: &mut Vec<String>, reply: &CheckpointReply) {
    fields.push(REPLY_OPCODE.to_string());
    match reply {
        CheckpointReply::OperationAccepted { operation_id } => {
            fields.push("operation_accepted".to_string());
            fields.push(operation_id.to_string());
        }
        CheckpointReply::Accepted => fields.push("accepted".to_string()),
        CheckpointReply::Checkpoints(checkpoints) => {
            fields.push("checkpoints".to_string());
            fields.push(to_json(checkpoints));
        }
        CheckpointReply::Operations(operations) => {
            fields.push("operations".to_string());
            fields.push(to_json(operations));
        }
        CheckpointReply::Operation(operation) => {
            fields.push("operation".to_string());
            fields.push(to_json(operation));
        }
        CheckpointReply::Status { held, busy } => {
            fields.push("status".to_string());
            fields.push(held.to_string());
            fields.push(busy.to_string());
        }
    }
}

pub(super) fn decode_reply(fields: &[String]) -> Result<CheckpointReply, ControlCodecError> {
    match field(fields, 1)?.as_str() {
        "operation_accepted" => Ok(CheckpointReply::OperationAccepted {
            operation_id: parse_u64(field(fields, 2)?, "operation_id")?,
        }),
        "accepted" => Ok(CheckpointReply::Accepted),
        "status" => Ok(CheckpointReply::Status {
            held: parse_bool(field(fields, 2)?, "held")?,
            busy: parse_bool(field(fields, 3)?, "busy")?,
        }),
        "checkpoints" => from_json(field(fields, 2)?).map(CheckpointReply::Checkpoints),
        "operations" => from_json(field(fields, 2)?).map(CheckpointReply::Operations),
        "operation" => from_json(field(fields, 2)?)
            .map(|operation| CheckpointReply::Operation(Box::new(operation))),
        _ => Err(ControlCodecError::new(
            "decode",
            "unknown checkpoint reply kind",
        )),
    }
}

/// Appended to the current `TrackAdd` wire fields.
pub(super) fn encode_launch_config(
    fields: &mut Vec<String>,
    config: Option<&LaunchCheckpointConfig>,
) {
    let Some(config) = config else {
        fields.push("false".to_string());
        return;
    };
    fields.push("true".to_string());
    fields.push(config.scopes.len().to_string());
    for scope in &config.scopes {
        fields.push(scope.id.clone());
        fields.push(scope.root.clone());
        fields.push(scope.backend_instance.clone());
        fields.push(scope.excludes.len().to_string());
        fields.extend(scope.excludes.iter().cloned());
    }
}

/// Returns the configuration and the index of the next field.
pub(super) fn decode_launch_config(
    fields: &[String],
    mut cursor: usize,
) -> Result<(Option<LaunchCheckpointConfig>, usize), ControlCodecError> {
    if !super::parse_bool(field(fields, cursor)?, "checkpoint_present")? {
        return Ok((None, cursor + 1));
    }
    let scope_count = parse_usize(field(fields, cursor + 1)?, "checkpoint_scope_count")?;
    cursor += 2;
    let mut scopes = Vec::new();
    for _ in 0..scope_count {
        let exclude_count = parse_usize(field(fields, cursor + 3)?, "checkpoint_exclude_count")?;
        let mut excludes = Vec::new();
        for offset in 0..exclude_count {
            excludes.push(field(fields, cursor + 4 + offset)?.clone());
        }
        scopes.push(LaunchCheckpointScope {
            id: field(fields, cursor)?.clone(),
            root: field(fields, cursor + 1)?.clone(),
            backend_instance: field(fields, cursor + 2)?.clone(),
            excludes,
        });
        cursor += 4 + exclude_count;
    }
    Ok((Some(LaunchCheckpointConfig { scopes }), cursor))
}

fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("checkpoint views serialize to JSON")
}

fn from_json<T: DeserializeOwned>(raw: &str) -> Result<T, ControlCodecError> {
    serde_json::from_str(raw).map_err(|error| ControlCodecError::new("decode", error.to_string()))
}
