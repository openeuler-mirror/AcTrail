//! Shared Unix-socket framing and socket-path support for control transport.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use control_contract::command::{
    ControlCommand, DoctorCommand, ListTracesCommand, RegisterSeccompListenerCommand,
    TrackAddCommand, TrackRemoveCommand,
};
use control_contract::reply::{
    ControlError, ControlReply, DoctorReply, TraceListItem, TrackAddReply,
};
use control_contract::selector::TraceSelector;
use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};
use model_core::process::{InitialSuppressedFd, SuppressedFdPurpose};
use model_core::trace::{TraceHealth, TraceLifecycleState};
use std::str::FromStr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlCodecError {
    pub stage: String,
    pub message: String,
}

impl ControlCodecError {
    fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

pub fn encode_command(command: &ControlCommand) -> Vec<u8> {
    let mut fields = Vec::new();
    match command {
        ControlCommand::TrackAdd(command) => {
            fields.push("track_add_v2".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.root_pid.to_string());
            fields.push(command.display_name.to_string());
            fields.push(command.profile_name.to_string());
            fields.push(command.launch_mode.to_string());
            fields.push(command.initial_suppressed_fds.len().to_string());
            for suppressed_fd in &command.initial_suppressed_fds {
                fields.push(suppressed_fd.fd.to_string());
                fields.push(suppressed_fd.purpose.as_str().to_string());
            }
            fields.push(command.tags.len().to_string());
            fields.extend(command.tags.iter().cloned());
        }
        ControlCommand::RegisterSeccompListener(command) => {
            fields.push("register_seccomp_listener".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.trace_id.get().to_string());
            fields.push(command.target_pid.to_string());
        }
        ControlCommand::TrackRemove(command) => {
            fields.push("track_remove".to_string());
            fields.push(command.request_id.get().to_string());
            encode_selector(&mut fields, &command.selector);
        }
        ControlCommand::ListTraces(command) => {
            fields.push("list_traces".to_string());
            fields.push(command.request_id.get().to_string());
            if let Some(selector) = &command.selector {
                fields.push("1".to_string());
                encode_selector(&mut fields, selector);
            } else {
                fields.push("0".to_string());
            }
        }
        ControlCommand::Doctor(command) => {
            fields.push("doctor".to_string());
            fields.push(command.request_id.get().to_string());
        }
    }
    encode_fields(&fields)
}

pub fn decode_command(bytes: &[u8]) -> Result<ControlCommand, ControlCodecError> {
    let fields = decode_fields(bytes)?;
    let opcode = field(&fields, 0)?.as_str();
    match opcode {
        "track_add" => {
            let request_id = RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?);
            let root_pid = parse_u32(field(&fields, 2)?, "root_pid")?;
            let display_name = TraceName::new(field(&fields, 3)?);
            let profile_name = ProfileName::new(field(&fields, 4)?);
            let launch_mode = parse_bool(field(&fields, 5)?, "launch_mode")?;
            let tag_count = parse_usize(field(&fields, 6)?, "tag_count")?;
            let mut tags = BTreeSet::new();
            for offset in 0..tag_count {
                tags.insert(field(&fields, 7 + offset)?.clone());
            }
            Ok(ControlCommand::TrackAdd(TrackAddCommand {
                request_id,
                root_pid,
                display_name,
                profile_name,
                tags,
                launch_mode,
                initial_suppressed_fds: Vec::new(),
            }))
        }
        "track_add_v2" => {
            let request_id = RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?);
            let root_pid = parse_u32(field(&fields, 2)?, "root_pid")?;
            let display_name = TraceName::new(field(&fields, 3)?);
            let profile_name = ProfileName::new(field(&fields, 4)?);
            let launch_mode = parse_bool(field(&fields, 5)?, "launch_mode")?;
            let suppressed_count = parse_usize(field(&fields, 6)?, "suppressed_fd_count")?;
            let mut cursor = 7;
            let mut initial_suppressed_fds = Vec::new();
            for _ in 0..suppressed_count {
                let fd = parse_i32(field(&fields, cursor)?, "suppressed_fd")?;
                let purpose = SuppressedFdPurpose::from_str(field(&fields, cursor + 1)?)
                    .map_err(|error| ControlCodecError::new("decode", error))?;
                initial_suppressed_fds.push(InitialSuppressedFd { fd, purpose });
                cursor += 2;
            }
            let tag_count = parse_usize(field(&fields, cursor)?, "tag_count")?;
            cursor += 1;
            let mut tags = BTreeSet::new();
            for offset in 0..tag_count {
                tags.insert(field(&fields, cursor + offset)?.clone());
            }
            Ok(ControlCommand::TrackAdd(TrackAddCommand {
                request_id,
                root_pid,
                display_name,
                profile_name,
                tags,
                launch_mode,
                initial_suppressed_fds,
            }))
        }
        "register_seccomp_listener" => Ok(ControlCommand::RegisterSeccompListener(
            RegisterSeccompListenerCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                trace_id: TraceId::new(parse_u64(field(&fields, 2)?, "trace_id")?),
                target_pid: parse_u32(field(&fields, 3)?, "target_pid")?,
                listener_fd: None,
            },
        )),
        "track_remove" => Ok(ControlCommand::TrackRemove(TrackRemoveCommand {
            request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
            selector: decode_selector(&fields, 2)?,
        })),
        "list_traces" => {
            let has_selector = field(&fields, 2)? == "1";
            let selector = if has_selector {
                Some(decode_selector(&fields, 3)?)
            } else {
                None
            };
            Ok(ControlCommand::ListTraces(ListTracesCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                selector,
            }))
        }
        "doctor" => Ok(ControlCommand::Doctor(DoctorCommand {
            request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
        })),
        _ => Err(ControlCodecError::new("decode", "unknown command opcode")),
    }
}

pub fn encode_reply(reply: &Result<ControlReply, ControlError>) -> Vec<u8> {
    let mut fields = Vec::new();
    match reply {
        Ok(ControlReply::TrackAdded(reply)) => {
            fields.push("reply_track_added".to_string());
            fields.push(reply.trace_id.get().to_string());
            fields.push(format!("{:?}", reply.lifecycle_state));
        }
        Ok(ControlReply::SeccompListenerRegistered) => {
            fields.push("reply_seccomp_listener_registered".to_string());
        }
        Ok(ControlReply::TrackRemoved) => fields.push("reply_track_removed".to_string()),
        Ok(ControlReply::TraceList(items)) => {
            fields.push("reply_trace_list".to_string());
            fields.push(items.len().to_string());
            for item in items {
                fields.push(item.trace_id.get().to_string());
                fields.push(item.display_name.to_string());
                fields.push(item.root_pid.to_string());
                fields.push(format!("{:?}", item.lifecycle_state));
                fields.push(format!("{:?}", item.health));
                fields.push(system_time_to_secs(item.created_at).to_string());
                fields.push(item.tags.len().to_string());
                fields.extend(item.tags.iter().cloned());
            }
        }
        Ok(ControlReply::Doctor(reply)) => {
            fields.push("reply_doctor".to_string());
            fields.push(reply.available_collectors.len().to_string());
            fields.extend(reply.available_collectors.iter().cloned());
            fields.push(reply.loaded_policy_plugins.len().to_string());
            fields.extend(reply.loaded_policy_plugins.iter().cloned());
            fields.push(reply.storage_ready.to_string());
        }
        Err(error) => {
            fields.push("error".to_string());
            fields.push(error.code.clone());
            fields.push(error.message.clone());
        }
    }
    encode_fields(&fields)
}

pub fn decode_reply(bytes: &[u8]) -> Result<Result<ControlReply, ControlError>, ControlCodecError> {
    let fields = decode_fields(bytes)?;
    match field(&fields, 0)?.as_str() {
        "reply_track_added" => Ok(Ok(ControlReply::TrackAdded(TrackAddReply {
            trace_id: TraceId::new(parse_u64(field(&fields, 1)?, "trace_id")?),
            lifecycle_state: parse_lifecycle(field(&fields, 2)?)?,
        }))),
        "reply_track_removed" => Ok(Ok(ControlReply::TrackRemoved)),
        "reply_seccomp_listener_registered" => Ok(Ok(ControlReply::SeccompListenerRegistered)),
        "reply_trace_list" => {
            let count = parse_usize(field(&fields, 1)?, "count")?;
            let mut items = Vec::new();
            let mut cursor = 2;
            for _ in 0..count {
                let trace_id = TraceId::new(parse_u64(field(&fields, cursor)?, "trace_id")?);
                let display_name = TraceName::new(field(&fields, cursor + 1)?);
                let root_pid = parse_u32(field(&fields, cursor + 2)?, "root_pid")?;
                let lifecycle_state = parse_lifecycle(field(&fields, cursor + 3)?)?;
                let health = parse_health(field(&fields, cursor + 4)?)?;
                let created_at = UNIX_EPOCH
                    + Duration::from_secs(parse_u64(field(&fields, cursor + 5)?, "created_at")?);
                let tag_count = parse_usize(field(&fields, cursor + 6)?, "tag_count")?;
                let mut tags = BTreeSet::new();
                for tag_index in 0..tag_count {
                    tags.insert(field(&fields, cursor + 7 + tag_index)?.clone());
                }
                items.push(TraceListItem {
                    trace_id,
                    display_name,
                    root_pid,
                    lifecycle_state,
                    health,
                    tags,
                    created_at,
                });
                cursor += 7 + tag_count;
            }
            Ok(Ok(ControlReply::TraceList(items)))
        }
        "reply_doctor" => {
            let collector_count = parse_usize(field(&fields, 1)?, "collector_count")?;
            let mut cursor = 2;
            let mut available_collectors = Vec::new();
            for _ in 0..collector_count {
                available_collectors.push(field(&fields, cursor)?.clone());
                cursor += 1;
            }
            let plugin_count = parse_usize(field(&fields, cursor)?, "plugin_count")?;
            cursor += 1;
            let mut loaded_policy_plugins = Vec::new();
            for _ in 0..plugin_count {
                loaded_policy_plugins.push(field(&fields, cursor)?.clone());
                cursor += 1;
            }
            let storage_ready = field(&fields, cursor)? == "true";
            Ok(Ok(ControlReply::Doctor(DoctorReply {
                available_collectors,
                loaded_policy_plugins,
                storage_ready,
            })))
        }
        "error" => Ok(Err(ControlError::new(
            field(&fields, 1)?,
            field(&fields, 2)?,
        ))),
        _ => Err(ControlCodecError::new("decode", "unknown reply opcode")),
    }
}

fn encode_selector(fields: &mut Vec<String>, selector: &TraceSelector) {
    match selector {
        TraceSelector::TraceId(trace_id) => {
            fields.push("trace_id".to_string());
            fields.push(trace_id.get().to_string());
        }
        TraceSelector::RootPid(root_pid) => {
            fields.push("root_pid".to_string());
            fields.push(root_pid.to_string());
        }
        TraceSelector::Tag(tag) => {
            fields.push("tag".to_string());
            fields.push(tag.clone());
        }
        TraceSelector::Name(name) => {
            fields.push("name".to_string());
            fields.push(name.to_string());
        }
    }
}

fn decode_selector(fields: &[String], offset: usize) -> Result<TraceSelector, ControlCodecError> {
    match field(fields, offset)?.as_str() {
        "trace_id" => Ok(TraceSelector::TraceId(TraceId::new(parse_u64(
            field(fields, offset + 1)?,
            "trace_id",
        )?))),
        "root_pid" => Ok(TraceSelector::RootPid(parse_u32(
            field(fields, offset + 1)?,
            "root_pid",
        )?)),
        "tag" => Ok(TraceSelector::Tag(field(fields, offset + 1)?.clone())),
        "name" => Ok(TraceSelector::Name(TraceName::new(field(
            fields,
            offset + 1,
        )?))),
        _ => Err(ControlCodecError::new("decode", "unknown selector kind")),
    }
}

fn encode_fields(fields: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(field.len().to_string().as_bytes());
        bytes.push(b'#');
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes
}

fn decode_fields(bytes: &[u8]) -> Result<Vec<String>, ControlCodecError> {
    let mut cursor = 0;
    let mut fields = Vec::new();
    while cursor < bytes.len() {
        let mut length = String::new();
        while cursor < bytes.len() && bytes[cursor] != b'#' {
            length.push(bytes[cursor] as char);
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return Err(ControlCodecError::new(
                "decode",
                "unterminated field length",
            ));
        }
        cursor += 1;
        let length = length
            .parse::<usize>()
            .map_err(|_| ControlCodecError::new("decode", "invalid field length"))?;
        if cursor + length > bytes.len() {
            return Err(ControlCodecError::new(
                "decode",
                "field exceeds frame length",
            ));
        }
        let field = String::from_utf8(bytes[cursor..cursor + length].to_vec())
            .map_err(|_| ControlCodecError::new("decode", "field is not utf8"))?;
        fields.push(field);
        cursor += length;
    }
    Ok(fields)
}

fn field<'a>(fields: &'a [String], index: usize) -> Result<&'a String, ControlCodecError> {
    fields
        .get(index)
        .ok_or_else(|| ControlCodecError::new("decode", "missing field"))
}

fn parse_u64(raw: &str, field_name: &str) -> Result<u64, ControlCodecError> {
    raw.parse()
        .map_err(|_| ControlCodecError::new("decode", format!("invalid {}", field_name)))
}

fn parse_u32(raw: &str, field_name: &str) -> Result<u32, ControlCodecError> {
    raw.parse()
        .map_err(|_| ControlCodecError::new("decode", format!("invalid {}", field_name)))
}

fn parse_i32(raw: &str, field_name: &str) -> Result<i32, ControlCodecError> {
    raw.parse()
        .map_err(|_| ControlCodecError::new("decode", format!("invalid {}", field_name)))
}

fn parse_usize(raw: &str, field_name: &str) -> Result<usize, ControlCodecError> {
    raw.parse()
        .map_err(|_| ControlCodecError::new("decode", format!("invalid {}", field_name)))
}

fn parse_bool(raw: &str, field_name: &str) -> Result<bool, ControlCodecError> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ControlCodecError::new(
            "decode",
            format!("invalid {}", field_name),
        )),
    }
}

fn parse_lifecycle(raw: &str) -> Result<TraceLifecycleState, ControlCodecError> {
    match raw {
        "Starting" => Ok(TraceLifecycleState::Starting),
        "Active" => Ok(TraceLifecycleState::Active),
        "Draining" => Ok(TraceLifecycleState::Draining),
        "Completed" => Ok(TraceLifecycleState::Completed),
        "Failed" => Ok(TraceLifecycleState::Failed),
        _ => Err(ControlCodecError::new("decode", "invalid lifecycle state")),
    }
}

fn parse_health(raw: &str) -> Result<TraceHealth, ControlCodecError> {
    match raw {
        "Clean" => Ok(TraceHealth::Clean),
        "Degraded" => Ok(TraceHealth::Degraded),
        _ => Err(ControlCodecError::new("decode", "invalid trace health")),
    }
}

fn system_time_to_secs(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_add_v2_round_trips_initial_suppressed_fds() {
        let command = ControlCommand::TrackAdd(TrackAddCommand {
            request_id: RequestId::new(7),
            root_pid: 42,
            display_name: TraceName::new("launch"),
            profile_name: ProfileName::new("default"),
            tags: BTreeSet::from(["agent".to_string()]),
            launch_mode: true,
            initial_suppressed_fds: vec![InitialSuppressedFd {
                fd: 3,
                purpose: SuppressedFdPurpose::TlsSyncEvent,
            }],
        });

        let decoded = decode_command(&encode_command(&command)).expect("decode command");

        assert_eq!(decoded, command);
    }
}
