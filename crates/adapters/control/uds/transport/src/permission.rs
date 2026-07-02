use std::str::FromStr;

use control_contract::command::{
    ControlCommand, DeploymentPermissionMode, ResolveLaunchPermissionsCommand,
};
use control_contract::reply::{ControlReply, LaunchPermissionsReply};
use model_core::capability::Capability;
use model_core::ids::{ProfileName, RequestId};

use super::{ControlCodecError, field, parse_bool, parse_u64, parse_usize};

pub(super) fn encode_command(fields: &mut Vec<String>, command: &ResolveLaunchPermissionsCommand) {
    fields.push("resolve_launch_permissions_v1".to_string());
    fields.push(command.request_id.get().to_string());
    fields.push(command.profile_name.to_string());
    fields.push(command.host_ebpf.as_str().to_string());
    fields.push(command.seccomp_notify.as_str().to_string());
    fields.push(command.seccomp_notify_available.to_string());
    fields.push(command.seccomp_notify_detail.clone());
}

pub(super) fn decode_command(fields: &[String]) -> Result<ControlCommand, ControlCodecError> {
    Ok(ControlCommand::ResolveLaunchPermissions(
        ResolveLaunchPermissionsCommand {
            request_id: RequestId::new(parse_u64(field(fields, 1)?, "request_id")?),
            profile_name: ProfileName::new(field(fields, 2)?),
            host_ebpf: parse_permission_mode(field(fields, 3)?)?,
            seccomp_notify: parse_permission_mode(field(fields, 4)?)?,
            seccomp_notify_available: parse_bool(field(fields, 5)?, "seccomp_notify_available")?,
            seccomp_notify_detail: field(fields, 6)?.clone(),
        },
    ))
}

pub(super) fn encode_reply(fields: &mut Vec<String>, reply: &LaunchPermissionsReply) {
    fields.push("reply_launch_permissions_v1".to_string());
    fields.push(reply.requested_host_ebpf.as_str().to_string());
    fields.push(reply.requested_seccomp_notify.as_str().to_string());
    fields.push(reply.selected_host_ebpf.to_string());
    fields.push(reply.selected_seccomp_notify.to_string());
    fields.push(reply.selected_profile_name.to_string());
    fields.push(reply.payload_tls_seccomp.to_string());
    fields.push(reply.payload_socket_seccomp.to_string());
    fields.push(reply.process_seccomp.to_string());
    fields.push(reply.network_control_seccomp.to_string());
    fields.push(reply.degraded.to_string());
    fields.push(reply.required_capabilities.len().to_string());
    fields.extend(
        reply
            .required_capabilities
            .iter()
            .map(|capability| capability.as_str().to_string()),
    );
    fields.push(reply.reasons.len().to_string());
    fields.extend(reply.reasons.iter().cloned());
}

pub(super) fn decode_reply(fields: &[String]) -> Result<ControlReply, ControlCodecError> {
    let capability_count = parse_usize(field(fields, 11)?, "required_capability_count")?;
    let mut cursor = 12;
    let mut required_capabilities = Vec::new();
    for _ in 0..capability_count {
        required_capabilities.push(
            Capability::from_str(field(fields, cursor)?)
                .map_err(|error| ControlCodecError::new("decode", error))?,
        );
        cursor += 1;
    }
    let reason_count = parse_usize(field(fields, cursor)?, "permission_reason_count")?;
    cursor += 1;
    let mut reasons = Vec::new();
    for _ in 0..reason_count {
        reasons.push(field(fields, cursor)?.clone());
        cursor += 1;
    }
    Ok(ControlReply::LaunchPermissions(LaunchPermissionsReply {
        requested_host_ebpf: parse_permission_mode(field(fields, 1)?)?,
        requested_seccomp_notify: parse_permission_mode(field(fields, 2)?)?,
        selected_host_ebpf: parse_bool(field(fields, 3)?, "selected_host_ebpf")?,
        selected_seccomp_notify: parse_bool(field(fields, 4)?, "selected_seccomp_notify")?,
        selected_profile_name: ProfileName::new(field(fields, 5)?),
        payload_tls_seccomp: parse_bool(field(fields, 6)?, "payload_tls_seccomp")?,
        payload_socket_seccomp: parse_bool(field(fields, 7)?, "payload_socket_seccomp")?,
        process_seccomp: parse_bool(field(fields, 8)?, "process_seccomp")?,
        network_control_seccomp: parse_bool(field(fields, 9)?, "network_control_seccomp")?,
        degraded: parse_bool(field(fields, 10)?, "degraded")?,
        required_capabilities,
        reasons,
    }))
}

fn parse_permission_mode(raw: &str) -> Result<DeploymentPermissionMode, ControlCodecError> {
    match raw {
        "auto" => Ok(DeploymentPermissionMode::Auto),
        "required" => Ok(DeploymentPermissionMode::Required),
        "disabled" => Ok(DeploymentPermissionMode::Disabled),
        _ => Err(ControlCodecError::new(
            "decode",
            "invalid deployment permission mode",
        )),
    }
}

#[cfg(test)]
mod tests {
    use control_contract::reply::ControlError;

    use super::*;
    use crate::{decode_command, decode_reply, encode_command, encode_reply};

    #[test]
    fn launch_permissions_v1_round_trips_command_and_reply() {
        let command = ControlCommand::ResolveLaunchPermissions(ResolveLaunchPermissionsCommand {
            request_id: RequestId::new(6),
            profile_name: ProfileName::new("container-auto"),
            host_ebpf: DeploymentPermissionMode::Auto,
            seccomp_notify: DeploymentPermissionMode::Required,
            seccomp_notify_available: false,
            seccomp_notify_detail: "pidfd_getfd denied".to_string(),
        });
        assert_eq!(
            decode_command(&encode_command(&command)).expect("decode command"),
            command
        );

        let reply: Result<ControlReply, ControlError> =
            Ok(ControlReply::LaunchPermissions(LaunchPermissionsReply {
                requested_host_ebpf: DeploymentPermissionMode::Auto,
                requested_seccomp_notify: DeploymentPermissionMode::Auto,
                selected_host_ebpf: true,
                selected_seccomp_notify: false,
                selected_profile_name: ProfileName::new("container-auto-ebpf-on-notify-off"),
                payload_tls_seccomp: false,
                payload_socket_seccomp: false,
                process_seccomp: false,
                network_control_seccomp: false,
                required_capabilities: vec![
                    Capability::ProcLifecycle,
                    Capability::SocketPlaintextPayload,
                ],
                degraded: true,
                reasons: vec!["seccomp_notify_unavailable: denied".to_string()],
            }));
        assert_eq!(
            decode_reply(&encode_reply(&reply)).expect("decode reply"),
            reply
        );
    }
}
