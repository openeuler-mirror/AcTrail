//! Host eBPF and workload seccomp-notify permission resolution for launch.

use config_core::capture_profile::{
    CaptureProfile, DeploymentPermissionAvailability, DeploymentPermissions,
    resolve_deployment_permissions as resolve_permission_decision,
};
use control_contract::command::DeploymentPermissionMode;
use control_contract::reply::LaunchPermissionsReply;

use crate::platform_probe::LaunchPlatformReport;

pub use config_core::capture_profile::{
    DeploymentPermissionPolicy, FileEnforcementSeccompRequirements, LaunchSeccompRequirements,
    PermissionDecision, PermissionMode,
};

pub fn contract_permission_mode(mode: PermissionMode) -> DeploymentPermissionMode {
    match mode {
        PermissionMode::Auto => DeploymentPermissionMode::Auto,
        PermissionMode::Required => DeploymentPermissionMode::Required,
        PermissionMode::Disabled => DeploymentPermissionMode::Disabled,
    }
}

pub fn permission_decision_from_reply(reply: &LaunchPermissionsReply) -> PermissionDecision {
    PermissionDecision {
        requested_host_ebpf: permission_mode(reply.requested_host_ebpf),
        requested_seccomp_notify: permission_mode(reply.requested_seccomp_notify),
        selected: DeploymentPermissions::new(
            reply.selected_host_ebpf,
            reply.selected_seccomp_notify,
        ),
        required_capabilities: reply.required_capabilities.clone(),
        degraded: reply.degraded,
        reasons: reply.reasons.clone(),
    }
}

fn permission_mode(mode: DeploymentPermissionMode) -> PermissionMode {
    match mode {
        DeploymentPermissionMode::Auto => PermissionMode::Auto,
        DeploymentPermissionMode::Required => PermissionMode::Required,
        DeploymentPermissionMode::Disabled => PermissionMode::Disabled,
    }
}

pub fn resolve_deployment_permissions(
    policy: DeploymentPermissionPolicy,
    configured_profile: &CaptureProfile,
    launch_seccomp_requirements: LaunchSeccompRequirements,
    probe: Option<&LaunchPlatformReport>,
) -> Result<PermissionDecision, String> {
    let availability = DeploymentPermissionAvailability {
        host_ebpf: probe
            .and_then(|report| report.daemon.as_ref())
            .map(|daemon| {
                daemon
                    .available_collectors
                    .iter()
                    .any(|collector| collector == "ebpf")
            }),
        seccomp_notify: probe.map(LaunchPlatformReport::seccomp_notify_available),
        seccomp_notify_detail: probe
            .map(|report| report.seccomp_notify.detail.clone())
            .unwrap_or_else(|| "platform probe was not run".to_string()),
    };
    resolve_permission_decision(
        policy,
        configured_profile,
        launch_seccomp_requirements,
        &availability,
    )
}
