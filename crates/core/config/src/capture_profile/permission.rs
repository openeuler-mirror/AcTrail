use model_core::capability::Capability;

use super::CaptureProfile;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeploymentPermissions {
    pub host_ebpf: bool,
    pub seccomp_notify: bool,
}

impl DeploymentPermissions {
    pub const ALL: [Self; 4] = [
        Self::new(false, false),
        Self::new(false, true),
        Self::new(true, false),
        Self::new(true, true),
    ];

    pub const fn new(host_ebpf: bool, seccomp_notify: bool) -> Self {
        Self {
            host_ebpf,
            seccomp_notify,
        }
    }

    pub(super) const fn profile_suffix(self) -> &'static str {
        match (self.host_ebpf, self.seccomp_notify) {
            (false, false) => "-ebpf-off-notify-off",
            (false, true) => "-ebpf-off-notify-on",
            (true, false) => "-ebpf-on-notify-off",
            (true, true) => "-ebpf-on-notify-on",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionMode {
    Auto,
    Required,
    Disabled,
}

impl PermissionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Required => "required",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeploymentPermissionPolicy {
    pub host_ebpf: PermissionMode,
    pub seccomp_notify: PermissionMode,
}

impl DeploymentPermissionPolicy {
    pub const fn auto() -> Self {
        Self {
            host_ebpf: PermissionMode::Auto,
            seccomp_notify: PermissionMode::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LaunchSeccompRequirements {
    pub payload_tls: bool,
    pub payload_socket: bool,
    pub process_seccomp: bool,
    pub network_control: bool,
}

impl LaunchSeccompRequirements {
    pub const fn new(
        payload_tls: bool,
        payload_socket: bool,
        process_seccomp: bool,
        network_control: bool,
    ) -> Self {
        Self {
            payload_tls,
            payload_socket,
            process_seccomp,
            network_control,
        }
    }

    pub const fn requires_seccomp_notify(self) -> bool {
        self.payload_tls || self.payload_socket || self.process_seccomp || self.network_control
    }

    pub const fn enabled_by(self, seccomp_notify: bool) -> Self {
        if seccomp_notify {
            self
        } else {
            Self::new(false, false, false, false)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentPermissionAvailability {
    pub host_ebpf: Option<bool>,
    pub seccomp_notify: Option<bool>,
    pub seccomp_notify_detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionDecision {
    pub requested_host_ebpf: PermissionMode,
    pub requested_seccomp_notify: PermissionMode,
    pub selected: DeploymentPermissions,
    pub required_capabilities: Vec<Capability>,
    pub degraded: bool,
    pub reasons: Vec<String>,
}

impl PermissionDecision {
    pub fn selected_profile(&self, configured: &CaptureProfile) -> CaptureProfile {
        configured.for_permissions(self.selected)
    }
}

pub fn resolve_deployment_permissions(
    policy: DeploymentPermissionPolicy,
    configured_profile: &CaptureProfile,
    launch_seccomp_requirements: LaunchSeccompRequirements,
    availability: &DeploymentPermissionAvailability,
) -> Result<PermissionDecision, String> {
    let host_ebpf_configured = configured_profile.supports_host_ebpf_observation();
    let seccomp_notify_configured = launch_seccomp_requirements.requires_seccomp_notify();
    let mut reasons = Vec::new();

    let selected_host_ebpf = match policy.host_ebpf {
        PermissionMode::Disabled => false,
        PermissionMode::Required if !host_ebpf_configured => {
            return Err(
                "host eBPF required but capture profile does not require proc-lifecycle and net-transport"
                    .to_string(),
            );
        }
        PermissionMode::Required => match availability.host_ebpf {
            Some(true) => true,
            Some(false) => {
                return Err("host eBPF required but daemon collector is unavailable".to_string());
            }
            None => {
                return Err(
                    "host eBPF status unavailable: daemon doctor did not return collector status"
                        .to_string(),
                );
            }
        },
        PermissionMode::Auto if !host_ebpf_configured => {
            reasons.push("host_ebpf_not_configured".to_string());
            false
        }
        PermissionMode::Auto => match availability.host_ebpf {
            Some(true) => true,
            Some(false) => {
                reasons.push("host_ebpf_unavailable".to_string());
                false
            }
            None => {
                return Err(
                    "host eBPF status unavailable: daemon doctor did not return collector status"
                        .to_string(),
                );
            }
        },
    };

    let selected_seccomp_notify = match policy.seccomp_notify {
        PermissionMode::Disabled => false,
        PermissionMode::Required if !seccomp_notify_configured => {
            return Err(
                "seccomp-notify required but launch config does not require seccomp-notify"
                    .to_string(),
            );
        }
        PermissionMode::Required => match availability.seccomp_notify {
            Some(true) => true,
            Some(false) => {
                return Err(format!(
                    "seccomp-notify required but unavailable: {}",
                    availability.seccomp_notify_detail
                ));
            }
            None => {
                return Err(
                    "seccomp-notify required but platform probe was not run".to_string(),
                );
            }
        },
        PermissionMode::Auto if !seccomp_notify_configured => {
            reasons.push("seccomp_notify_not_configured".to_string());
            false
        }
        PermissionMode::Auto => match availability.seccomp_notify {
            Some(true) => true,
            Some(false) => {
                reasons.push(format!(
                    "seccomp_notify_unavailable: {}",
                    availability.seccomp_notify_detail
                ));
                false
            }
            None => {
                return Err(
                    "seccomp-notify status unavailable: platform probe was not run".to_string(),
                );
            }
        },
    };

    let selected = DeploymentPermissions::new(selected_host_ebpf, selected_seccomp_notify);
    let degraded = (policy.host_ebpf == PermissionMode::Auto
        && host_ebpf_configured
        && !selected_host_ebpf)
        || (policy.seccomp_notify == PermissionMode::Auto
            && seccomp_notify_configured
            && !selected_seccomp_notify);

    Ok(PermissionDecision {
        requested_host_ebpf: policy.host_ebpf,
        requested_seccomp_notify: policy.seccomp_notify,
        selected,
        required_capabilities: configured_profile
            .required_capabilities_for_permissions(selected),
        degraded,
        reasons,
    })
}
