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
        [Capability::ProcLifecycle, Capability::NetTransport]
            .iter()
            .all(|capability| {
                self.capabilities.iter().any(|request| {
                    request.capability == *capability && request.mode == RequestMode::Required
                })
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
    fn host_ebpf_contract_requires_process_and_network_observation() {
        let incomplete = CaptureProfile::new(
            ProfileName::new("incomplete"),
            vec![CapabilityRequest::new(
                Capability::ProcLifecycle,
                RequestMode::Required,
            )],
        );
        assert!(!incomplete.supports_host_ebpf_observation());

        let host_observe = CaptureProfile::new(
            ProfileName::new("host-observe"),
            vec![
                CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
                CapabilityRequest::new(Capability::NetTransport, RequestMode::Required),
            ],
        );
        assert!(host_observe.supports_host_ebpf_observation());
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
