//! Shared Unix-socket framing and socket-path support for control transport.

mod permission;
mod plugin;
mod reply;

pub use reply::{decode_reply, encode_reply};

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_lifecycle_contract::{TurnLifecycleKind, UserInteractionState, WorkKind, WorkState};
use control_contract::command::{
    ControlCommand, DoctorCommand, LaunchTlsProbePlan, ListTracesCommand, PluginCommandCommand,
    PluginConfigGetCommand, PluginConfigUpdateCommand, PluginConfigValidateCommand,
    PluginListCommand, PluginLoadCommand, PluginStatusCommand, PluginUnloadCommand, ProcessRef,
    RegisterSeccompListenerCommand, ReportSessionClosedCommand, ReportTurnLifecycleCommand,
    ReportUserInteractionCommand, ReportWorkLifecycleCommand, ResolveLaunchTlsPlanCommand,
    TrackAddCommand, TrackRemoveCommand,
};
use control_contract::selector::TraceSelector;
use model_core::binary_identity::{BinaryIdentity, BinaryIdentityTypeCode};
use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};
use model_core::process::{InitialSuppressedFd, NamespaceIdentity, SuppressedFdPurpose};
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
        ControlCommand::ResolveLaunchPermissions(command) => {
            permission::encode_command(&mut fields, command);
        }
        ControlCommand::ResolveLaunchTlsPlan(command) => {
            fields.push("resolve_launch_tls_plan_v1".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.binary.display().to_string());
        }
        ControlCommand::TrackAdd(command) => {
            fields.push(
                if command.launch_mode {
                    "track_add_v6"
                } else {
                    "track_add_v5"
                }
                .to_string(),
            );
            fields.push(command.request_id.get().to_string());
            encode_process_ref(&mut fields, &command.root);
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
            fields.push(command.tags.len().to_string());
            fields.extend(command.tags.iter().cloned());
        }
        ControlCommand::RegisterSeccompListener(command) => {
            fields.push("register_seccomp_listener_v2".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.trace_id.get().to_string());
            encode_process_ref(&mut fields, &command.target);
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
        ControlCommand::ReportTurnLifecycle(command) => {
            fields.push("report_turn_lifecycle_v2".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.trace_id.get().to_string());
            fields.push(command.session_id.clone());
            fields.push(command.task_id.clone());
            fields.push(command.kind.as_str().to_string());
            fields.push(system_time_to_nanos(command.observed_at).to_string());
        }
        ControlCommand::ReportUserInteraction(command) => {
            fields.push("report_user_interaction_v2".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.trace_id.get().to_string());
            fields.push(command.session_id.clone());
            fields.push(command.task_id.clone());
            fields.push(command.interaction_id.clone());
            fields.push(command.state.as_str().to_string());
            fields.push(system_time_to_nanos(command.observed_at).to_string());
        }
        ControlCommand::ReportWorkLifecycle(command) => {
            fields.push("report_work_lifecycle".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.trace_id.get().to_string());
            fields.push(command.session_id.clone());
            fields.push(command.task_id.clone());
            fields.push(command.kind.as_str().to_string());
            fields.push(command.state.as_str().to_string());
            fields.push(system_time_to_nanos(command.observed_at).to_string());
        }
        ControlCommand::ReportSessionClosed(command) => {
            fields.push("report_session_closed".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.trace_id.get().to_string());
            fields.push(command.session_id.clone());
            fields.push(system_time_to_nanos(command.observed_at).to_string());
        }
        ControlCommand::PluginList(command) => {
            fields.push("plugin_list".to_string());
            fields.push(command.request_id.get().to_string());
        }
        ControlCommand::PluginStatus(command) => {
            fields.push("plugin_status".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.instance_id.clone());
        }
        ControlCommand::PluginLoad(command) => {
            fields.push("plugin_load".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.manifest_path.clone());
            if let Some(plugin_config_path) = &command.plugin_config_path {
                fields.push("1".to_string());
                fields.push(plugin_config_path.clone());
            } else {
                fields.push("0".to_string());
            }
            fields.push(command.instance_id.clone());
            fields.push(command.host_grants.len().to_string());
            fields.extend(command.host_grants.iter().cloned());
        }
        ControlCommand::PluginUnload(command) => {
            fields.push("plugin_unload".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.instance_id.clone());
        }
        ControlCommand::PluginCommand(command) => {
            fields.push("plugin_cmd_v1".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.instance_id.clone());
            fields.push(command.argv.len().to_string());
            fields.extend(command.argv.iter().cloned());
        }
        ControlCommand::PluginConfigGet(command) => {
            fields.push("plugin_config_get_v1".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.instance_id.clone());
        }
        ControlCommand::PluginConfigValidate(command) => {
            fields.push("plugin_config_validate_v1".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.instance_id.clone());
            fields.push(command.config_json.clone());
        }
        ControlCommand::PluginConfigUpdate(command) => {
            fields.push("plugin_config_update_v1".to_string());
            fields.push(command.request_id.get().to_string());
            fields.push(command.instance_id.clone());
            fields.push(command.config_json.clone());
        }
    }
    encode_fields(&fields)
}

pub fn decode_command(bytes: &[u8]) -> Result<ControlCommand, ControlCodecError> {
    let fields = decode_fields(bytes)?;
    let opcode = field(&fields, 0)?.as_str();
    match opcode {
        "resolve_launch_permissions_v2" => permission::decode_command(&fields),
        "resolve_launch_tls_plan_v1" => Ok(ControlCommand::ResolveLaunchTlsPlan(
            ResolveLaunchTlsPlanCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                binary: PathBuf::from(field(&fields, 2)?),
            },
        )),
        "track_add" => {
            let request_id = RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?);
            let root = unknown_process_ref(parse_u32(field(&fields, 2)?, "root_pid")?);
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
                root,
                display_name,
                profile_name,
                tags,
                launch_mode,
                initial_suppressed_fds: Vec::new(),
                tls_probe_plans: Vec::new(),
            }))
        }
        "track_add_v2" => {
            let request_id = RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?);
            let root = unknown_process_ref(parse_u32(field(&fields, 2)?, "root_pid")?);
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
                root,
                display_name,
                profile_name,
                tags,
                launch_mode,
                initial_suppressed_fds,
                tls_probe_plans: Vec::new(),
            }))
        }
        "track_add_v3" => {
            let request_id = RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?);
            let root = decode_process_ref(&fields, 2)?;
            let display_name = TraceName::new(field(&fields, 4)?);
            let profile_name = ProfileName::new(field(&fields, 5)?);
            let launch_mode = parse_bool(field(&fields, 6)?, "launch_mode")?;
            let suppressed_count = parse_usize(field(&fields, 7)?, "suppressed_fd_count")?;
            let mut cursor = 8;
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
                root,
                display_name,
                profile_name,
                tags,
                launch_mode,
                initial_suppressed_fds,
                tls_probe_plans: Vec::new(),
            }))
        }
        "track_add_v4" | "track_add_v5" => {
            let request_id = RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?);
            let root = decode_process_ref(&fields, 2)?;
            let display_name = TraceName::new(field(&fields, 4)?);
            let profile_name = ProfileName::new(field(&fields, 5)?);
            let launch_mode = parse_bool(field(&fields, 6)?, "launch_mode")?;
            let suppressed_count = parse_usize(field(&fields, 7)?, "suppressed_fd_count")?;
            let mut cursor = 8;
            let mut initial_suppressed_fds = Vec::new();
            for _ in 0..suppressed_count {
                let fd = parse_i32(field(&fields, cursor)?, "suppressed_fd")?;
                let purpose = SuppressedFdPurpose::from_str(field(&fields, cursor + 1)?)
                    .map_err(|error| ControlCodecError::new("decode", error))?;
                initial_suppressed_fds.push(InitialSuppressedFd { fd, purpose });
                cursor += 2;
            }
            let mut tls_probe_plans = Vec::new();
            if field(&fields, cursor)? == "1" {
                cursor += 1;
                let plan = LaunchTlsProbePlan {
                    target: PathBuf::from(field(&fields, cursor)?),
                    target_identity: decode_binary_identity(&fields, cursor + 1, "target")?,
                    binary: PathBuf::from(field(&fields, cursor + 3)?),
                    binary_identity: decode_binary_identity(&fields, cursor + 4, "binary")?,
                    provider: field(&fields, cursor + 6)?.clone(),
                    points: field(&fields, cursor + 7)?.clone(),
                };
                cursor += 8;
                tls_probe_plans.push(plan);
            } else {
                cursor += 1;
            }
            let tag_count = parse_usize(field(&fields, cursor)?, "tag_count")?;
            cursor += 1;
            let mut tags = BTreeSet::new();
            for offset in 0..tag_count {
                tags.insert(field(&fields, cursor + offset)?.clone());
            }
            Ok(ControlCommand::TrackAdd(TrackAddCommand {
                request_id,
                root,
                display_name,
                profile_name,
                tags,
                launch_mode,
                initial_suppressed_fds,
                tls_probe_plans,
            }))
        }
        "track_add_v6" => {
            let request_id = RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?);
            let root = decode_process_ref(&fields, 2)?;
            let display_name = TraceName::new(field(&fields, 4)?);
            let profile_name = ProfileName::new(field(&fields, 5)?);
            let launch_mode = parse_bool(field(&fields, 6)?, "launch_mode")?;
            let suppressed_count = parse_usize(field(&fields, 7)?, "suppressed_fd_count")?;
            let mut cursor = 8;
            let mut initial_suppressed_fds = Vec::new();
            for _ in 0..suppressed_count {
                let fd = parse_i32(field(&fields, cursor)?, "suppressed_fd")?;
                let purpose = SuppressedFdPurpose::from_str(field(&fields, cursor + 1)?)
                    .map_err(|error| ControlCodecError::new("decode", error))?;
                initial_suppressed_fds.push(InitialSuppressedFd { fd, purpose });
                cursor += 2;
            }
            let plan_count = parse_usize(field(&fields, cursor)?, "tls_probe_plan_count")?;
            cursor += 1;
            let mut tls_probe_plans = Vec::with_capacity(plan_count);
            for _ in 0..plan_count {
                let plan = LaunchTlsProbePlan {
                    target: PathBuf::from(field(&fields, cursor)?),
                    target_identity: decode_binary_identity(&fields, cursor + 1, "target")?,
                    binary: PathBuf::from(field(&fields, cursor + 3)?),
                    binary_identity: decode_binary_identity(&fields, cursor + 4, "binary")?,
                    provider: field(&fields, cursor + 6)?.clone(),
                    points: field(&fields, cursor + 7)?.clone(),
                };
                tls_probe_plans.push(plan);
                cursor += 8;
            }
            let tag_count = parse_usize(field(&fields, cursor)?, "tag_count")?;
            cursor += 1;
            let mut tags = BTreeSet::new();
            for offset in 0..tag_count {
                tags.insert(field(&fields, cursor + offset)?.clone());
            }
            Ok(ControlCommand::TrackAdd(TrackAddCommand {
                request_id,
                root,
                display_name,
                profile_name,
                tags,
                launch_mode,
                initial_suppressed_fds,
                tls_probe_plans,
            }))
        }
        "register_seccomp_listener" => Ok(ControlCommand::RegisterSeccompListener(
            RegisterSeccompListenerCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                trace_id: TraceId::new(parse_u64(field(&fields, 2)?, "trace_id")?),
                target: unknown_process_ref(parse_u32(field(&fields, 3)?, "target_pid")?),
                listener_fd: None,
            },
        )),
        "register_seccomp_listener_v2" => Ok(ControlCommand::RegisterSeccompListener(
            RegisterSeccompListenerCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                trace_id: TraceId::new(parse_u64(field(&fields, 2)?, "trace_id")?),
                target: decode_process_ref(&fields, 3)?,
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
        "plugin_list" => Ok(ControlCommand::PluginList(PluginListCommand {
            request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
        })),
        "plugin_status" => Ok(ControlCommand::PluginStatus(PluginStatusCommand {
            request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
            instance_id: field(&fields, 2)?.clone(),
        })),
        "plugin_load" => {
            let has_plugin_config = field(&fields, 3)? == "1";
            let (plugin_config_path, instance_offset) = if has_plugin_config {
                (Some(field(&fields, 4)?.clone()), 5)
            } else {
                (None, 4)
            };
            let grant_offset = instance_offset + 1;
            let host_grants = if fields.len() > grant_offset {
                let grant_count = parse_usize(field(&fields, grant_offset)?, "plugin_grant_count")?;
                let mut host_grants = Vec::new();
                for offset in 0..grant_count {
                    host_grants.push(field(&fields, grant_offset + 1 + offset)?.clone());
                }
                host_grants
            } else {
                Vec::new()
            };
            Ok(ControlCommand::PluginLoad(PluginLoadCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                manifest_path: field(&fields, 2)?.clone(),
                plugin_config_path,
                instance_id: field(&fields, instance_offset)?.clone(),
                host_grants,
            }))
        }
        "plugin_unload" => Ok(ControlCommand::PluginUnload(PluginUnloadCommand {
            request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
            instance_id: field(&fields, 2)?.clone(),
        })),
        "plugin_cmd_v1" => {
            let arg_count = parse_usize(field(&fields, 3)?, "plugin_command_arg_count")?;
            let mut argv = Vec::new();
            for offset in 0..arg_count {
                argv.push(field(&fields, 4 + offset)?.clone());
            }
            Ok(ControlCommand::PluginCommand(PluginCommandCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                instance_id: field(&fields, 2)?.clone(),
                argv,
            }))
        }
        "plugin_config_get_v1" => Ok(ControlCommand::PluginConfigGet(PluginConfigGetCommand {
            request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
            instance_id: field(&fields, 2)?.clone(),
        })),
        "plugin_config_validate_v1" => Ok(ControlCommand::PluginConfigValidate(
            PluginConfigValidateCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                instance_id: field(&fields, 2)?.clone(),
                config_json: field(&fields, 3)?.clone(),
            },
        )),
        "plugin_config_update_v1" => Ok(ControlCommand::PluginConfigUpdate(
            PluginConfigUpdateCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                instance_id: field(&fields, 2)?.clone(),
                config_json: field(&fields, 3)?.clone(),
            },
        )),
        "report_turn_lifecycle_v2" => Ok(ControlCommand::ReportTurnLifecycle(
            ReportTurnLifecycleCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                trace_id: TraceId::new(parse_u64(field(&fields, 2)?, "trace_id")?),
                session_id: field(&fields, 3)?.clone(),
                task_id: field(&fields, 4)?.clone(),
                kind: TurnLifecycleKind::parse(field(&fields, 5)?).ok_or_else(|| {
                    ControlCodecError::new("decode", "invalid turn lifecycle kind")
                })?,
                observed_at: nanos_to_system_time(parse_u64(
                    field(&fields, 6)?,
                    "observed_at_unix_nanos",
                )?)?,
            },
        )),
        "report_user_interaction_v2" => Ok(ControlCommand::ReportUserInteraction(
            ReportUserInteractionCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                trace_id: TraceId::new(parse_u64(field(&fields, 2)?, "trace_id")?),
                session_id: field(&fields, 3)?.clone(),
                task_id: field(&fields, 4)?.clone(),
                interaction_id: field(&fields, 5)?.clone(),
                state: UserInteractionState::parse(field(&fields, 6)?).ok_or_else(|| {
                    ControlCodecError::new("decode", "invalid user interaction state")
                })?,
                observed_at: nanos_to_system_time(parse_u64(
                    field(&fields, 7)?,
                    "observed_at_unix_nanos",
                )?)?,
            },
        )),
        "report_work_lifecycle" => Ok(ControlCommand::ReportWorkLifecycle(
            ReportWorkLifecycleCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                trace_id: TraceId::new(parse_u64(field(&fields, 2)?, "trace_id")?),
                session_id: field(&fields, 3)?.clone(),
                task_id: field(&fields, 4)?.clone(),
                kind: WorkKind::parse(field(&fields, 5)?)
                    .ok_or_else(|| ControlCodecError::new("decode", "invalid work kind"))?,
                state: WorkState::parse(field(&fields, 6)?)
                    .ok_or_else(|| ControlCodecError::new("decode", "invalid work state"))?,
                observed_at: nanos_to_system_time(parse_u64(
                    field(&fields, 7)?,
                    "observed_at_unix_nanos",
                )?)?,
            },
        )),
        "report_session_closed" => Ok(ControlCommand::ReportSessionClosed(
            ReportSessionClosedCommand {
                request_id: RequestId::new(parse_u64(field(&fields, 1)?, "request_id")?),
                trace_id: TraceId::new(parse_u64(field(&fields, 2)?, "trace_id")?),
                session_id: field(&fields, 3)?.clone(),
                observed_at: nanos_to_system_time(parse_u64(
                    field(&fields, 4)?,
                    "observed_at_unix_nanos",
                )?)?,
            },
        )),
        _ => Err(ControlCodecError::new("decode", "unknown command opcode")),
    }
}

fn decode_binary_identity(
    fields: &[String],
    type_index: usize,
    label: &str,
) -> Result<BinaryIdentity, ControlCodecError> {
    let raw_code = parse_u64(
        field(fields, type_index)?,
        &format!("{label}_identity_type_code"),
    )?;
    let code = u16::try_from(raw_code).map_err(|_| {
        ControlCodecError::new("decode", format!("{label} identity type code is too large"))
    })?;
    let identity_type_code = BinaryIdentityTypeCode::parse(code)
        .map_err(|error| ControlCodecError::new("decode", error.to_string()))?;
    BinaryIdentity::try_new(identity_type_code, field(fields, type_index + 1)?.clone())
        .map_err(|error| ControlCodecError::new("decode", error.to_string()))
}

fn encode_process_ref(fields: &mut Vec<String>, process: &ProcessRef) {
    fields.push(process.namespace_pid.to_string());
    fields.push(process.pid_namespace.as_str().to_string());
}

fn decode_process_ref(fields: &[String], offset: usize) -> Result<ProcessRef, ControlCodecError> {
    Ok(ProcessRef::new(
        parse_u32(field(fields, offset)?, "namespace_pid")?,
        NamespaceIdentity::new(field(fields, offset + 1)?.clone()),
    ))
}

fn unknown_process_ref(pid: u32) -> ProcessRef {
    ProcessRef::new(pid, NamespaceIdentity::new("unknown"))
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
    TraceLifecycleState::from_display_str(raw)
        .ok_or_else(|| ControlCodecError::new("decode", "invalid lifecycle state"))
}

fn parse_health(raw: &str) -> Result<TraceHealth, ControlCodecError> {
    match raw {
        "Clean" => Ok(TraceHealth::Clean),
        "Degraded" => Ok(TraceHealth::Degraded),
        _ => Err(ControlCodecError::new("decode", "invalid trace health")),
    }
}

fn system_time_to_nanos(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn nanos_to_system_time(nanos: u64) -> Result<SystemTime, ControlCodecError> {
    UNIX_EPOCH
        .checked_add(Duration::from_nanos(nanos))
        .ok_or_else(|| ControlCodecError::new("decode", "observed_at is out of range"))
}
