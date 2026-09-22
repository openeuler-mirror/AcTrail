//! Control reply encoding and decoding.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use control_contract::reply::{
    ControlError, ControlReply, DoctorReply, LaunchTlsPlanDescriptor, LaunchTlsPlanReply,
    LaunchTlsPlanStatus, LaunchTlsPlanUnavailableReason, PluginCommandReply, PluginConfigReply,
    PluginConfigValidationReply, TraceListItem, TrackAddReply,
};
use model_core::ids::{TraceId, TraceName};
use model_core::process::NamespaceIdentity;

use super::plugin::{
    decode_plugin_status_v1, decode_plugin_status_v2, decode_plugin_statuses_v1,
    decode_plugin_statuses_v2, encode_plugin_status_v2, encode_plugin_statuses_v2,
};
use super::{
    ControlCodecError, decode_binary_identity, decode_fields, encode_fields, field, parse_bool,
    parse_health, parse_i32, parse_lifecycle, parse_u32, parse_u64, parse_usize, permission,
};

fn system_time_to_secs(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub fn encode_reply(reply: &Result<ControlReply, ControlError>) -> Vec<u8> {
    let mut fields = Vec::new();
    match reply {
        Ok(ControlReply::LaunchPermissions(reply)) => {
            permission::encode_reply(&mut fields, reply);
        }
        Ok(ControlReply::LaunchTlsPlan(reply)) => {
            fields.push("reply_launch_tls_plan_v3".to_string());
            fields.push(reply.cache_hit.to_string());
            fields.push(reply.resolve_elapsed_micros.to_string());
            match &reply.status {
                LaunchTlsPlanStatus::Found(plans) => {
                    fields.push("found".to_string());
                    fields.push(plans.len().to_string());
                    for plan in plans {
                        fields.push(plan.target.display().to_string());
                        fields.push(plan.target_identity.identity_type_code.code().to_string());
                        fields.push(plan.target_identity.identity.clone());
                        fields.push(plan.binary.display().to_string());
                        fields.push(plan.binary_identity.identity_type_code.code().to_string());
                        fields.push(plan.binary_identity.identity.clone());
                        fields.push(plan.provider.clone());
                        fields.push(plan.source.clone());
                        fields.push(plan.points.clone());
                    }
                }
                LaunchTlsPlanStatus::Unsupported { reason } => {
                    fields.push("unsupported".to_string());
                    fields.push(reason.code().to_string());
                }
            }
        }
        Ok(ControlReply::TrackAdded(reply)) => {
            fields.push("reply_track_added".to_string());
            fields.push(reply.trace_id.get().to_string());
            fields.push(reply.lifecycle_state.as_display_str().to_string());
        }
        Ok(ControlReply::SeccompListenerRegistered) => {
            fields.push("reply_seccomp_listener_registered".to_string());
        }
        Ok(ControlReply::TrackRemoved) => fields.push("reply_track_removed".to_string()),
        Ok(ControlReply::TurnLifecycleRecorded) => {
            fields.push("reply_turn_lifecycle_recorded".to_string());
        }
        Ok(ControlReply::WorkLifecycleRecorded) => {
            fields.push("reply_work_lifecycle_recorded".to_string());
        }
        Ok(ControlReply::SessionClosedRecorded) => {
            fields.push("reply_session_closed_recorded".to_string());
        }
        Ok(ControlReply::UserInteractionRecorded) => {
            fields.push("reply_user_interaction_recorded".to_string());
        }
        Ok(ControlReply::TraceList(items)) => {
            fields.push("reply_trace_list_v3".to_string());
            fields.push(items.len().to_string());
            for item in items {
                fields.push(item.trace_id.get().to_string());
                fields.push(item.display_name.to_string());
                fields.push(item.root_pid.to_string());
                fields.push(
                    item.root_pid_namespace
                        .as_ref()
                        .map(|namespace| namespace.as_str())
                        .unwrap_or_default()
                        .to_string(),
                );
                fields.push(item.root_container_id.clone().unwrap_or_default());
                fields.push(item.lifecycle_state.as_display_str().to_string());
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
        Ok(ControlReply::PluginList(items)) => {
            fields.push("reply_plugin_list_v2".to_string());
            encode_plugin_statuses_v2(&mut fields, items);
        }
        Ok(ControlReply::PluginStatus(status)) => {
            fields.push("reply_plugin_status_v2".to_string());
            encode_plugin_status_v2(&mut fields, status);
        }
        Ok(ControlReply::PluginCommand(reply)) => {
            fields.push("reply_plugin_command_v1".to_string());
            fields.push(reply.instance_id.clone());
            fields.push(reply.exit_code.to_string());
            fields.push(reply.stdout.clone());
            fields.push(reply.stderr.clone());
        }
        Ok(ControlReply::PluginConfig(reply)) => {
            fields.push("reply_plugin_config_v1".to_string());
            fields.push(reply.instance_id.clone());
            fields.push(reply.plugin_id.clone());
            fields.push(reply.editable.to_string());
            fields.push(reply.config_json.clone());
            fields.push(reply.schema_json.clone());
        }
        Ok(ControlReply::PluginConfigValidation(reply)) => {
            fields.push("reply_plugin_config_validation_v1".to_string());
            fields.push(reply.instance_id.clone());
            fields.push(reply.valid.to_string());
            fields.push(reply.errors.len().to_string());
            fields.extend(reply.errors.iter().cloned());
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
        "reply_launch_permissions_v2" => permission::decode_reply(&fields).map(Ok),
        "reply_launch_tls_plan_v2" => {
            let status = match field(&fields, 3)?.as_str() {
                "found" => LaunchTlsPlanStatus::Found(vec![LaunchTlsPlanDescriptor {
                    target: PathBuf::from(field(&fields, 4)?),
                    target_identity: decode_binary_identity(&fields, 5, "target")?,
                    binary: PathBuf::from(field(&fields, 7)?),
                    binary_identity: decode_binary_identity(&fields, 8, "binary")?,
                    provider: field(&fields, 10)?.clone(),
                    source: field(&fields, 11)?.clone(),
                    points: field(&fields, 12)?.clone(),
                }]),
                "unsupported" => LaunchTlsPlanStatus::Unsupported {
                    reason: decode_tls_plan_reason(field(&fields, 4)?)?,
                },
                _ => {
                    return Err(ControlCodecError::new(
                        "decode",
                        "invalid launch TLS plan status",
                    ));
                }
            };
            Ok(Ok(ControlReply::LaunchTlsPlan(LaunchTlsPlanReply {
                cache_hit: parse_bool(field(&fields, 1)?, "cache_hit")?,
                resolve_elapsed_micros: parse_u64(field(&fields, 2)?, "resolve_elapsed_micros")?,
                status,
            })))
        }
        "reply_launch_tls_plan_v3" => {
            let status = match field(&fields, 3)?.as_str() {
                "found" => {
                    let count = parse_usize(field(&fields, 4)?, "tls_plan_count")?;
                    let mut cursor = 5;
                    let mut plans = Vec::with_capacity(count);
                    for _ in 0..count {
                        let plan = LaunchTlsPlanDescriptor {
                            target: PathBuf::from(field(&fields, cursor)?),
                            target_identity: decode_binary_identity(&fields, cursor + 1, "target")?,
                            binary: PathBuf::from(field(&fields, cursor + 3)?),
                            binary_identity: decode_binary_identity(&fields, cursor + 4, "binary")?,
                            provider: field(&fields, cursor + 6)?.clone(),
                            source: field(&fields, cursor + 7)?.clone(),
                            points: field(&fields, cursor + 8)?.clone(),
                        };
                        plans.push(plan);
                        cursor += 9;
                    }
                    LaunchTlsPlanStatus::Found(plans)
                }
                "unsupported" => LaunchTlsPlanStatus::Unsupported {
                    reason: decode_tls_plan_reason(field(&fields, 4)?)?,
                },
                _ => {
                    return Err(ControlCodecError::new(
                        "decode",
                        "invalid launch TLS plan status",
                    ));
                }
            };
            Ok(Ok(ControlReply::LaunchTlsPlan(LaunchTlsPlanReply {
                cache_hit: parse_bool(field(&fields, 1)?, "cache_hit")?,
                resolve_elapsed_micros: parse_u64(field(&fields, 2)?, "resolve_elapsed_micros")?,
                status,
            })))
        }
        "reply_track_added" => Ok(Ok(ControlReply::TrackAdded(TrackAddReply {
            trace_id: TraceId::new(parse_u64(field(&fields, 1)?, "trace_id")?),
            lifecycle_state: parse_lifecycle(field(&fields, 2)?)?,
        }))),
        "reply_track_removed" => Ok(Ok(ControlReply::TrackRemoved)),
        "reply_turn_lifecycle_recorded" => Ok(Ok(ControlReply::TurnLifecycleRecorded)),
        "reply_user_interaction_recorded" => Ok(Ok(ControlReply::UserInteractionRecorded)),
        "reply_work_lifecycle_recorded" => Ok(Ok(ControlReply::WorkLifecycleRecorded)),
        "reply_session_closed_recorded" => Ok(Ok(ControlReply::SessionClosedRecorded)),
        "reply_seccomp_listener_registered" => Ok(Ok(ControlReply::SeccompListenerRegistered)),
        "reply_trace_list" | "reply_trace_list_v2" | "reply_trace_list_v3" => {
            let trace_list_version = fields[0].as_str();
            let count = parse_usize(field(&fields, 1)?, "count")?;
            let mut items = Vec::new();
            let mut cursor = 2;
            for _ in 0..count {
                let trace_id = TraceId::new(parse_u64(field(&fields, cursor)?, "trace_id")?);
                let display_name = TraceName::new(field(&fields, cursor + 1)?);
                let root_pid = parse_u32(field(&fields, cursor + 2)?, "root_pid")?;
                let includes_pid_namespace = matches!(
                    trace_list_version,
                    "reply_trace_list_v2" | "reply_trace_list_v3"
                );
                let includes_container_id = trace_list_version == "reply_trace_list_v3";
                let root_pid_namespace = if includes_pid_namespace {
                    match field(&fields, cursor + 3)?.as_str() {
                        "" => None,
                        value => Some(NamespaceIdentity::new(value)),
                    }
                } else {
                    None
                };
                let root_container_id = if includes_container_id {
                    let container_offset = usize::from(includes_pid_namespace);
                    match field(&fields, cursor + 3 + container_offset)?.as_str() {
                        "" => None,
                        value => Some(value.to_string()),
                    }
                } else {
                    None
                };
                let metadata_offset =
                    usize::from(includes_pid_namespace) + usize::from(includes_container_id);
                let lifecycle_state =
                    parse_lifecycle(field(&fields, cursor + 3 + metadata_offset)?)?;
                let health = parse_health(field(&fields, cursor + 4 + metadata_offset)?)?;
                let created_at = UNIX_EPOCH
                    + Duration::from_secs(parse_u64(
                        field(&fields, cursor + 5 + metadata_offset)?,
                        "created_at",
                    )?);
                let tag_count =
                    parse_usize(field(&fields, cursor + 6 + metadata_offset)?, "tag_count")?;
                let mut tags = BTreeSet::new();
                for tag_index in 0..tag_count {
                    tags.insert(field(&fields, cursor + 7 + metadata_offset + tag_index)?.clone());
                }
                items.push(TraceListItem {
                    trace_id,
                    display_name,
                    root_pid,
                    root_pid_namespace,
                    root_container_id,
                    lifecycle_state,
                    health,
                    tags,
                    created_at,
                });
                cursor += 7 + metadata_offset + tag_count;
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
        "reply_plugin_list" => {
            let (items, _) = decode_plugin_statuses_v1(&fields, 1)?;
            Ok(Ok(ControlReply::PluginList(items)))
        }
        "reply_plugin_list_v2" => {
            let (items, _) = decode_plugin_statuses_v2(&fields, 1)?;
            Ok(Ok(ControlReply::PluginList(items)))
        }
        "reply_plugin_status" => {
            let (status, _) = decode_plugin_status_v1(&fields, 1)?;
            Ok(Ok(ControlReply::PluginStatus(status)))
        }
        "reply_plugin_status_v2" => {
            let (status, _) = decode_plugin_status_v2(&fields, 1)?;
            Ok(Ok(ControlReply::PluginStatus(status)))
        }
        "reply_plugin_command_v1" => Ok(Ok(ControlReply::PluginCommand(PluginCommandReply {
            instance_id: field(&fields, 1)?.clone(),
            exit_code: parse_i32(field(&fields, 2)?, "exit_code")?,
            stdout: field(&fields, 3)?.clone(),
            stderr: field(&fields, 4)?.clone(),
        }))),
        "reply_plugin_config_v1" => Ok(Ok(ControlReply::PluginConfig(PluginConfigReply {
            instance_id: field(&fields, 1)?.clone(),
            plugin_id: field(&fields, 2)?.clone(),
            editable: parse_bool(field(&fields, 3)?, "editable")?,
            config_json: field(&fields, 4)?.clone(),
            schema_json: field(&fields, 5)?.clone(),
        }))),
        "reply_plugin_config_validation_v1" => {
            let error_count = parse_usize(field(&fields, 3)?, "plugin_config_error_count")?;
            let mut errors = Vec::with_capacity(error_count);
            for offset in 0..error_count {
                errors.push(field(&fields, 4 + offset)?.clone());
            }
            Ok(Ok(ControlReply::PluginConfigValidation(
                PluginConfigValidationReply {
                    instance_id: field(&fields, 1)?.clone(),
                    valid: parse_bool(field(&fields, 2)?, "valid")?,
                    errors,
                },
            )))
        }
        "error" => Ok(Err(ControlError::new(
            field(&fields, 1)?,
            field(&fields, 2)?,
        ))),
        _ => Err(ControlCodecError::new("decode", "unknown reply opcode")),
    }
}

fn decode_tls_plan_reason(raw: &str) -> Result<LaunchTlsPlanUnavailableReason, ControlCodecError> {
    raw.parse::<u8>()
        .ok()
        .and_then(|code| LaunchTlsPlanUnavailableReason::try_from(code).ok())
        .ok_or_else(|| ControlCodecError::new("decode", "invalid launch TLS plan reason code"))
}
