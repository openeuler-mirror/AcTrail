//! Capture-profile declarations used to build immutable trace snapshots.

#[path = "capture_profile/permission.rs"]
mod permission;

use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::ProfileName;

pub use permission::{
    DeploymentPermissionAvailability, DeploymentPermissionPolicy, DeploymentPermissions,
    LaunchSeccompRequirements, PermissionDecision, PermissionMode, resolve_deployment_permissions,
};

/// Capabilities that only the host eBPF collector can provide. Dropped on the
/// profiles where host eBPF is disabled.
fn is_ebpf_only_capability(capability: &Capability) -> bool {
    matches!(
        capability,
        Capability::ProcLifecycle
            | Capability::FsAccessBasic
            | Capability::FsMmap
            | Capability::FsExecAccess
            | Capability::NetTransport
            | Capability::NetDns
            | Capability::NetTlsMetadata
            | Capability::NetProviderClassification
            | Capability::IpcUnixSocket
            | Capability::IpcPipeFifo
            | Capability::StdioChunk
            | Capability::SocketPlaintextPayload
    )
}

/// Capabilities that need the seccomp-notify launch path.
fn is_seccomp_only_capability(capability: &Capability) -> bool {
    matches!(capability, Capability::ProcExecContext)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureDisposition {
    DegradeTrace,
    FailTrace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureProfile {
    pub name: ProfileName,
    pub capabilities: Vec<CapabilityRequest>,
    pub classify_providers: bool,
    pub enable_payload_collectors: bool,
    pub identity_failure: FailureDisposition,
    pub runtime_loss: FailureDisposition,
}

impl CaptureProfile {
    pub fn new(name: ProfileName, capabilities: Vec<CapabilityRequest>) -> Self {
        Self {
            name,
            capabilities,
            classify_providers: false,
            enable_payload_collectors: false,
            identity_failure: FailureDisposition::DegradeTrace,
            runtime_loss: FailureDisposition::DegradeTrace,
        }
    }

    pub fn supports_host_ebpf_observation(&self) -> bool {
        self.capabilities.iter().any(|request| {
            request.mode != RequestMode::Disabled && is_ebpf_only_capability(&request.capability)
        })
    }

    pub fn for_permissions(&self, permissions: DeploymentPermissions) -> Self {
        let mut profile = self.clone();
        profile.name = ProfileName::new(format!(
            "{}{}",
            self.name.as_str(),
            permissions.profile_suffix()
        ));
        profile.capabilities.retain(|request| {
            (permissions.host_ebpf || !is_ebpf_only_capability(&request.capability))
                && (permissions.seccomp_notify || !is_seccomp_only_capability(&request.capability))
        });
        profile
    }

    pub fn required_capabilities_for_permissions(
        &self,
        permissions: DeploymentPermissions,
    ) -> Vec<Capability> {
        self.for_permissions(permissions)
            .capabilities
            .into_iter()
            .filter_map(|request| {
                (request.mode == RequestMode::Required).then_some(request.capability)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_enabled_ebpf_capability_requests_host_observation() {
        let file_scan = CaptureProfile::new(
            ProfileName::new("file-scan"),
            vec![CapabilityRequest::new(
                Capability::FsAccessBasic,
                RequestMode::Required,
            )],
        );
        assert!(file_scan.supports_host_ebpf_observation());

        let opportunistic_dns = CaptureProfile::new(
            ProfileName::new("opportunistic-dns"),
            vec![CapabilityRequest::new(
                Capability::NetDns,
                RequestMode::Opportunistic,
            )],
        );
        assert!(opportunistic_dns.supports_host_ebpf_observation());

        let disabled_ebpf = CaptureProfile::new(
            ProfileName::new("disabled-ebpf"),
            vec![CapabilityRequest::new(
                Capability::ProcLifecycle,
                RequestMode::Disabled,
            )],
        );
        assert!(!disabled_ebpf.supports_host_ebpf_observation());

        let tls_only = CaptureProfile::new(
            ProfileName::new("tls-only"),
            vec![CapabilityRequest::new(
                Capability::TlsPlaintextPayload,
                RequestMode::Required,
            )],
        );
        assert!(!tls_only.supports_host_ebpf_observation());
    }

    #[test]
    fn partial_ebpf_profile_participates_in_auto_permission_resolution() {
        let profile = CaptureProfile::new(
            ProfileName::new("file-scan"),
            vec![CapabilityRequest::new(
                Capability::FsAccessBasic,
                RequestMode::Required,
            )],
        );

        let decision = resolve_deployment_permissions(
            DeploymentPermissionPolicy::auto(),
            &profile,
            LaunchSeccompRequirements::default(),
            &DeploymentPermissionAvailability {
                host_ebpf: Some(true),
                seccomp_notify: Some(false),
                seccomp_notify_detail: "not needed".to_string(),
            },
        )
        .expect("partial eBPF profile should resolve");

        assert!(decision.selected.host_ebpf);
        assert!(!decision.selected.seccomp_notify);
        assert!(!decision.degraded);
        assert_eq!(
            decision.required_capabilities,
            vec![Capability::FsAccessBasic]
        );
    }

    #[test]
    fn permission_combinations_drop_backend_specific_capabilities() {
        let profile = CaptureProfile::new(
            ProfileName::new("container-auto"),
            vec![
                CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
                CapabilityRequest::new(Capability::NetTransport, RequestMode::Required),
                CapabilityRequest::new(Capability::TlsPlaintextPayload, RequestMode::Required),
                CapabilityRequest::new(Capability::ProcExecContext, RequestMode::Required),
                CapabilityRequest::new(
                    Capability::SocketPlaintextPayload,
                    RequestMode::Opportunistic,
                ),
                CapabilityRequest::new(Capability::ResourceMetrics, RequestMode::Opportunistic),
            ],
        );
        let has =
            |p: &CaptureProfile, c: Capability| p.capabilities.iter().any(|r| r.capability == c);

        let neither = profile.for_permissions(DeploymentPermissions::new(false, false));
        assert_eq!(neither.name.as_str(), "container-auto-ebpf-off-notify-off");
        assert!(has(&neither, Capability::TlsPlaintextPayload));
        assert!(has(&neither, Capability::ResourceMetrics));
        assert!(!has(&neither, Capability::ProcLifecycle));
        assert!(!has(&neither, Capability::NetTransport));
        assert!(!has(&neither, Capability::ProcExecContext));
        assert!(!has(&neither, Capability::SocketPlaintextPayload));

        let ebpf_only = profile.for_permissions(DeploymentPermissions::new(true, false));
        assert_eq!(ebpf_only.name.as_str(), "container-auto-ebpf-on-notify-off");
        assert!(has(&ebpf_only, Capability::ProcLifecycle));
        assert!(has(&ebpf_only, Capability::NetTransport));
        assert!(!has(&ebpf_only, Capability::ProcExecContext));
        assert!(has(&ebpf_only, Capability::SocketPlaintextPayload));

        let notify_only = profile.for_permissions(DeploymentPermissions::new(false, true));
        assert_eq!(
            notify_only.name.as_str(),
            "container-auto-ebpf-off-notify-on"
        );
        assert!(has(&notify_only, Capability::TlsPlaintextPayload));
        assert!(!has(&notify_only, Capability::ProcLifecycle));
        assert!(has(&notify_only, Capability::ProcExecContext));
        assert!(!has(&notify_only, Capability::SocketPlaintextPayload));

        let both = profile.for_permissions(DeploymentPermissions::new(true, true));
        assert_eq!(both.name.as_str(), "container-auto-ebpf-on-notify-on");
        assert!(has(&both, Capability::ProcLifecycle));
        assert!(has(&both, Capability::ProcExecContext));
        assert!(has(&both, Capability::SocketPlaintextPayload));
    }
}
