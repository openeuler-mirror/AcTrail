//! Track registration wire fields.
use super::{
    ControlCodecError, decode_binary_identity, decode_process_ref, encode_process_ref, field,
    parse_bool, parse_i32, parse_u32, parse_u64, parse_usize, unknown_process_ref,
};
use control_contract::command::{
    ControlCommand, LaunchTarget, LaunchTlsProbePlan, TrackAddCommand,
};
use model_core::ids::{ProfileName, RequestId, TraceName};
use model_core::process::{InitialSuppressedFd, SuppressedFdPurpose};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;

pub(super) struct TrackCodec;
impl TrackCodec {
    pub(super) fn encode(fields: &mut Vec<String>, command: &TrackAddCommand) {
        fields.push("track_add_v7".to_string());
        fields.push(command.request_id.get().to_string());
        encode_process_ref(fields, &command.root);
        fields.push(command.display_name.to_string());
        fields.push(command.profile_name.to_string());
        fields.push(command.launch_mode.to_string());
        fields.push(command.initial_suppressed_fds.len().to_string());
        for suppressed_fd in &command.initial_suppressed_fds {
            fields.push(suppressed_fd.fd.to_string());
            fields.push(suppressed_fd.purpose.as_str().to_string());
        }
        fields.push(command.tls_probe_plans.len().to_string());
        for plan in &command.tls_probe_plans {
            fields.push(plan.target.display().to_string());
            fields.push(plan.target_identity.identity_type_code.code().to_string());
            fields.push(plan.target_identity.identity.clone());
            fields.push(plan.binary.display().to_string());
            fields.push(plan.binary_identity.identity_type_code.code().to_string());
            fields.push(plan.binary_identity.identity.clone());
            fields.push(plan.provider.clone());
            fields.push(plan.points.clone());
        }
        fields.push(command.launch_target.is_some().to_string());
        if let Some(target) = &command.launch_target {
            fields.push(target.executable.display().to_string());
            fields.push(target.candidates.len().to_string());
            fields.extend(
                target
                    .candidates
                    .iter()
                    .map(|path| path.display().to_string()),
            );
        }
        fields.push(command.tags.len().to_string());
        fields.extend(command.tags.iter().cloned());
    }
    pub(super) fn decode(fields: &[String]) -> Result<ControlCommand, ControlCodecError> {
        let opcode = field(fields, 0)?.as_str();
        match opcode {
            "track_add" => {
                let request_id = RequestId::new(parse_u64(field(fields, 1)?, "request_id")?);
                let root = unknown_process_ref(parse_u32(field(fields, 2)?, "root_pid")?);
                let display_name = TraceName::new(field(fields, 3)?);
                let profile_name = ProfileName::new(field(fields, 4)?);
                let launch_mode = parse_bool(field(fields, 5)?, "launch_mode")?;
                let tag_count = parse_usize(field(fields, 6)?, "tag_count")?;
                let mut tags = BTreeSet::new();
                for offset in 0..tag_count {
                    tags.insert(field(fields, 7 + offset)?.clone());
                }
                Ok(ControlCommand::TrackAdd(TrackAddCommand {
                    request_id,
                    root,
                    display_name,
                    profile_name,
                    tags,
                    launch_mode,
                    launch_target: None,
                    initial_suppressed_fds: Vec::new(),
                    tls_probe_plans: Vec::new(),
                }))
            }
            "track_add_v2" => {
                let request_id = RequestId::new(parse_u64(field(fields, 1)?, "request_id")?);
                let root = unknown_process_ref(parse_u32(field(fields, 2)?, "root_pid")?);
                let display_name = TraceName::new(field(fields, 3)?);
                let profile_name = ProfileName::new(field(fields, 4)?);
                let launch_mode = parse_bool(field(fields, 5)?, "launch_mode")?;
                let suppressed_count = parse_usize(field(fields, 6)?, "suppressed_fd_count")?;
                let mut cursor = 7;
                let mut initial_suppressed_fds = Vec::new();
                for _ in 0..suppressed_count {
                    let fd = parse_i32(field(fields, cursor)?, "suppressed_fd")?;
                    let purpose = SuppressedFdPurpose::from_str(field(fields, cursor + 1)?)
                        .map_err(|error| ControlCodecError::new("decode", error))?;
                    initial_suppressed_fds.push(InitialSuppressedFd { fd, purpose });
                    cursor += 2;
                }
                let tag_count = parse_usize(field(fields, cursor)?, "tag_count")?;
                cursor += 1;
                let mut tags = BTreeSet::new();
                for offset in 0..tag_count {
                    tags.insert(field(fields, cursor + offset)?.clone());
                }
                Ok(ControlCommand::TrackAdd(TrackAddCommand {
                    request_id,
                    root,
                    display_name,
                    profile_name,
                    tags,
                    launch_mode,
                    launch_target: None,
                    initial_suppressed_fds,
                    tls_probe_plans: Vec::new(),
                }))
            }
            "track_add_v3" => {
                let request_id = RequestId::new(parse_u64(field(fields, 1)?, "request_id")?);
                let root = decode_process_ref(fields, 2)?;
                let display_name = TraceName::new(field(fields, 4)?);
                let profile_name = ProfileName::new(field(fields, 5)?);
                let launch_mode = parse_bool(field(fields, 6)?, "launch_mode")?;
                let suppressed_count = parse_usize(field(fields, 7)?, "suppressed_fd_count")?;
                let mut cursor = 8;
                let mut initial_suppressed_fds = Vec::new();
                for _ in 0..suppressed_count {
                    let fd = parse_i32(field(fields, cursor)?, "suppressed_fd")?;
                    let purpose = SuppressedFdPurpose::from_str(field(fields, cursor + 1)?)
                        .map_err(|error| ControlCodecError::new("decode", error))?;
                    initial_suppressed_fds.push(InitialSuppressedFd { fd, purpose });
                    cursor += 2;
                }
                let tag_count = parse_usize(field(fields, cursor)?, "tag_count")?;
                cursor += 1;
                let mut tags = BTreeSet::new();
                for offset in 0..tag_count {
                    tags.insert(field(fields, cursor + offset)?.clone());
                }
                Ok(ControlCommand::TrackAdd(TrackAddCommand {
                    request_id,
                    root,
                    display_name,
                    profile_name,
                    tags,
                    launch_mode,
                    launch_target: None,
                    initial_suppressed_fds,
                    tls_probe_plans: Vec::new(),
                }))
            }
            "track_add_v4" | "track_add_v5" => {
                let request_id = RequestId::new(parse_u64(field(fields, 1)?, "request_id")?);
                let root = decode_process_ref(fields, 2)?;
                let display_name = TraceName::new(field(fields, 4)?);
                let profile_name = ProfileName::new(field(fields, 5)?);
                let launch_mode = parse_bool(field(fields, 6)?, "launch_mode")?;
                let suppressed_count = parse_usize(field(fields, 7)?, "suppressed_fd_count")?;
                let mut cursor = 8;
                let mut initial_suppressed_fds = Vec::new();
                for _ in 0..suppressed_count {
                    let fd = parse_i32(field(fields, cursor)?, "suppressed_fd")?;
                    let purpose = SuppressedFdPurpose::from_str(field(fields, cursor + 1)?)
                        .map_err(|error| ControlCodecError::new("decode", error))?;
                    initial_suppressed_fds.push(InitialSuppressedFd { fd, purpose });
                    cursor += 2;
                }
                let mut tls_probe_plans = Vec::new();
                if field(fields, cursor)? == "1" {
                    cursor += 1;
                    let plan = LaunchTlsProbePlan {
                        target: PathBuf::from(field(fields, cursor)?),
                        target_identity: decode_binary_identity(fields, cursor + 1, "target")?,
                        binary: PathBuf::from(field(fields, cursor + 3)?),
                        binary_identity: decode_binary_identity(fields, cursor + 4, "binary")?,
                        provider: field(fields, cursor + 6)?.clone(),
                        points: field(fields, cursor + 7)?.clone(),
                    };
                    cursor += 8;
                    tls_probe_plans.push(plan);
                } else {
                    cursor += 1;
                }
                let tag_count = parse_usize(field(fields, cursor)?, "tag_count")?;
                cursor += 1;
                let mut tags = BTreeSet::new();
                for offset in 0..tag_count {
                    tags.insert(field(fields, cursor + offset)?.clone());
                }
                Ok(ControlCommand::TrackAdd(TrackAddCommand {
                    request_id,
                    root,
                    display_name,
                    profile_name,
                    tags,
                    launch_mode,
                    launch_target: None,
                    initial_suppressed_fds,
                    tls_probe_plans,
                }))
            }
            "track_add_v6" | "track_add_v7" => {
                let request_id = RequestId::new(parse_u64(field(fields, 1)?, "request_id")?);
                let root = decode_process_ref(fields, 2)?;
                let display_name = TraceName::new(field(fields, 4)?);
                let profile_name = ProfileName::new(field(fields, 5)?);
                let launch_mode = parse_bool(field(fields, 6)?, "launch_mode")?;
                let suppressed_count = parse_usize(field(fields, 7)?, "suppressed_fd_count")?;
                let mut cursor = 8;
                let mut initial_suppressed_fds = Vec::new();
                for _ in 0..suppressed_count {
                    let fd = parse_i32(field(fields, cursor)?, "suppressed_fd")?;
                    let purpose = SuppressedFdPurpose::from_str(field(fields, cursor + 1)?)
                        .map_err(|error| ControlCodecError::new("decode", error))?;
                    initial_suppressed_fds.push(InitialSuppressedFd { fd, purpose });
                    cursor += 2;
                }
                let plan_count = parse_usize(field(fields, cursor)?, "tls_probe_plan_count")?;
                cursor += 1;
                let mut tls_probe_plans = Vec::with_capacity(plan_count);
                for _ in 0..plan_count {
                    let plan = LaunchTlsProbePlan {
                        target: PathBuf::from(field(fields, cursor)?),
                        target_identity: decode_binary_identity(fields, cursor + 1, "target")?,
                        binary: PathBuf::from(field(fields, cursor + 3)?),
                        binary_identity: decode_binary_identity(fields, cursor + 4, "binary")?,
                        provider: field(fields, cursor + 6)?.clone(),
                        points: field(fields, cursor + 7)?.clone(),
                    };
                    tls_probe_plans.push(plan);
                    cursor += 8;
                }
                let launch_target = if opcode == "track_add_v7" {
                    let present = parse_bool(field(fields, cursor)?, "launch_target_present")?;
                    cursor += 1;
                    if present {
                        let executable = PathBuf::from(field(fields, cursor)?);
                        let count =
                            parse_usize(field(fields, cursor + 1)?, "launch_candidate_count")?;
                        cursor += 2;
                        let mut candidates = Vec::new();
                        for _ in 0..count {
                            candidates.push(PathBuf::from(field(fields, cursor)?));
                            cursor += 1;
                        }
                        Some(LaunchTarget {
                            executable,
                            candidates,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let tag_count = parse_usize(field(fields, cursor)?, "tag_count")?;
                cursor += 1;
                let mut tags = BTreeSet::new();
                for offset in 0..tag_count {
                    tags.insert(field(fields, cursor + offset)?.clone());
                }
                Ok(ControlCommand::TrackAdd(TrackAddCommand {
                    request_id,
                    root,
                    display_name,
                    profile_name,
                    tags,
                    launch_mode,
                    launch_target,
                    initial_suppressed_fds,
                    tls_probe_plans,
                }))
            }
            _ => Err(ControlCodecError::new(
                "decode",
                "unknown track registration opcode",
            )),
        }
    }
}
