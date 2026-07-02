//! Host eBPF and workload seccomp-notify permission resolution for launch.

use config_core::capture_profile::{
    CaptureProfile, DeploymentPermissionAvailability, DeploymentPermissions,
    resolve_deployment_permissions as resolve_permission_decision,
};
use control_contract::command::DeploymentPermissionMode;
use control_contract::reply::LaunchPermissionsReply;

use crate::platform_probe::LaunchPlatformReport;

pub use config_core::capture_profile::{
    DeploymentPermissionPolicy, LaunchSeccompRequirements, PermissionDecision, PermissionMode,
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

#[cfg(test)]
mod tests {
    use config_core::capture_profile::CaptureProfile;
    use control_contract::reply::DoctorReply;
    use linux_platform::capability_probe::CapabilityStatus;
    use model_core::capability::{Capability, CapabilityRequest, RequestMode};
    use model_core::ids::ProfileName;

    use super::*;

    fn report(host_ebpf_available: bool, seccomp_available: bool) -> LaunchPlatformReport {
        LaunchPlatformReport {
            control_socket: CapabilityStatus::ok("control_socket", "ok"),
            tls_sync_socket: CapabilityStatus::ok("tls_sync_socket", "ok"),
            no_new_privs: CapabilityStatus::ok("no_new_privs", "ok"),
            seccomp_notify: if seccomp_available {
                CapabilityStatus::ok("seccomp_notify", "ok")
            } else {
                CapabilityStatus::unavailable("seccomp_notify", "pidfd_getfd denied")
            },
            tls_sync_runtime_library: CapabilityStatus::ok("tls_sync_runtime_library", "ok"),
            daemon: Some(DoctorReply {
                available_collectors: if host_ebpf_available {
                    vec!["ebpf".to_string(), "tls-sync".to_string()]
                } else {
                    vec!["tls-sync".to_string()]
                },
                loaded_policy_plugins: Vec::new(),
                storage_ready: true,
            }),
        }
    }

    fn full_profile() -> CaptureProfile {
        CaptureProfile::new(
            ProfileName::new("container-auto"),
            vec![
                CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
                CapabilityRequest::new(Capability::NetTransport, RequestMode::Required),
                CapabilityRequest::new(Capability::ProcExecContext, RequestMode::Required),
                CapabilityRequest::new(
                    Capability::SocketPlaintextPayload,
                    RequestMode::Required,
                ),
                CapabilityRequest::new(Capability::FsAccessBasic, RequestMode::Opportunistic),
            ],
        )
    }

    #[test]
    fn auto_enables_both_permissions_when_available() {
        let decision = resolve_deployment_permissions(
            DeploymentPermissionPolicy::auto(),
            &full_profile(),
            LaunchSeccompRequirements::new(true, true, true, true),
            Some(&report(true, true)),
        )
        .unwrap();

        assert_eq!(decision.selected, DeploymentPermissions::new(true, true));
        assert!(!decision.degraded);
    }

    #[test]
    fn auto_selects_only_host_ebpf_when_notify_is_unavailable() {
        let decision = resolve_deployment_permissions(
            DeploymentPermissionPolicy::auto(),
            &full_profile(),
            LaunchSeccompRequirements::new(true, true, true, true),
            Some(&report(true, false)),
        )
        .unwrap();

        assert_eq!(decision.selected, DeploymentPermissions::new(true, false));
        assert!(decision.degraded);
        assert!(decision.reasons[0].starts_with("seccomp_notify_unavailable:"));
    }

    #[test]
    fn auto_selects_only_notify_when_host_ebpf_is_unavailable() {
        let decision = resolve_deployment_permissions(
            DeploymentPermissionPolicy::auto(),
            &full_profile(),
            LaunchSeccompRequirements::new(true, true, true, true),
            Some(&report(false, true)),
        )
        .unwrap();

        assert_eq!(decision.selected, DeploymentPermissions::new(false, true));
        assert!(decision.degraded);
        assert_eq!(decision.reasons, vec!["host_ebpf_unavailable"]);
    }

    #[test]
    fn auto_disables_both_permissions_when_neither_is_available() {
        let decision = resolve_deployment_permissions(
            DeploymentPermissionPolicy::auto(),
            &full_profile(),
            LaunchSeccompRequirements::new(true, true, true, true),
            Some(&report(false, false)),
        )
        .unwrap();

        assert_eq!(decision.selected, DeploymentPermissions::new(false, false));
        assert!(decision.degraded);
        assert_eq!(decision.reasons.len(), 2);
    }

    #[test]
    fn auto_requires_daemon_status_when_host_observation_is_configured() {
        let error = resolve_deployment_permissions(
            DeploymentPermissionPolicy::auto(),
            &full_profile(),
            LaunchSeccompRequirements::new(true, true, true, true),
            Some(&LaunchPlatformReport {
                daemon: None,
                ..report(true, true)
            }),
        )
        .unwrap_err();

        assert!(error.contains("daemon doctor"));
    }

    #[test]
    fn required_host_ebpf_fails_when_daemon_does_not_offer_it() {
        let error = resolve_deployment_permissions(
            DeploymentPermissionPolicy {
                host_ebpf: PermissionMode::Required,
                seccomp_notify: PermissionMode::Disabled,
            },
            &full_profile(),
            LaunchSeccompRequirements::new(true, true, true, true),
            Some(&report(false, true)),
        )
        .unwrap_err();

        assert!(error.contains("host eBPF required"));
    }

    #[test]
    fn required_seccomp_notify_fails_when_platform_denies_it() {
        let error = resolve_deployment_permissions(
            DeploymentPermissionPolicy {
                host_ebpf: PermissionMode::Disabled,
                seccomp_notify: PermissionMode::Required,
            },
            &full_profile(),
            LaunchSeccompRequirements::new(true, true, true, true),
            Some(&report(true, false)),
        )
        .unwrap_err();

        assert!(error.contains("seccomp-notify required"));
        assert!(error.contains("pidfd_getfd denied"));
    }

    #[test]
    fn auto_does_not_enable_permissions_missing_from_config() {
        let profile = CaptureProfile::new(
            ProfileName::new("tls-only"),
            vec![CapabilityRequest::new(
                Capability::TlsPlaintextPayload,
                RequestMode::Required,
            )],
        );
        let decision = resolve_deployment_permissions(
            DeploymentPermissionPolicy::auto(),
            &profile,
            LaunchSeccompRequirements::default(),
            Some(&report(true, true)),
        )
        .unwrap();

        assert_eq!(decision.selected, DeploymentPermissions::new(false, false));
        assert!(!decision.degraded);
        assert_eq!(
            decision.reasons,
            vec!["host_ebpf_not_configured", "seccomp_notify_not_configured"]
        );
    }

    #[test]
    fn explicitly_disabled_notify_is_not_degradation_and_preserves_ebpf_payload() {
        let profile = full_profile();
        let decision = resolve_deployment_permissions(
            DeploymentPermissionPolicy {
                host_ebpf: PermissionMode::Required,
                seccomp_notify: PermissionMode::Disabled,
            },
            &profile,
            LaunchSeccompRequirements::new(true, true, true, true),
            Some(&report(true, true)),
        )
        .unwrap();
        let selected = decision.selected_profile(&profile);

        assert_eq!(decision.selected, DeploymentPermissions::new(true, false));
        assert!(!decision.degraded);
        assert!(!selected.capabilities.iter().any(
            |request| request.capability == Capability::ProcExecContext
        ));
        assert!(selected.capabilities.iter().any(
            |request| request.capability == Capability::SocketPlaintextPayload
        ));
        assert!(selected.capabilities.iter().any(|request| {
            request.capability == Capability::FsAccessBasic
                && request.mode == RequestMode::Opportunistic
        }));
    }
}
