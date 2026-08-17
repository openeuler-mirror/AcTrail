//! Capture-profile declarations used to build immutable trace snapshots.

#[path = "capture_profile/permission.rs"]
mod permission;

use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::ProfileName;

pub use permission::{
    DeploymentPermissionAvailability, DeploymentPermissionPolicy, DeploymentPermissions,
    FileEnforcementSeccompRequirements, LaunchSeccompRequirements, PermissionDecision,
    PermissionMode, resolve_deployment_permissions,
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
            | Capability::EnforcementFilePermissionFanotify
    )
}

/// Capabilities that need the seccomp-notify launch path.
fn is_seccomp_only_capability(capability: &Capability) -> bool {
    matches!(
        capability,
        Capability::ProcExecContext | Capability::EnforcementCommandExecutionSeccomp
    )
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
                && (!matches!(
                    request.capability,
                    Capability::EnforcementCommandExecutionSeccomp
                ) || (permissions.host_ebpf && permissions.seccomp_notify))
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
