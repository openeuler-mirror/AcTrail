//! Per-operation access-control routing and bounded backend configuration.

use super::enforcement::EnforcementSeccompSyscall;
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessControlBackend {
    Seccomp,
    #[default]
    Lsm,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileOpenBackend {
    Fanotify,
    #[default]
    Lsm,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OperationConfig<B: Default> {
    pub enabled: bool,
    pub backend: B,
}

impl<B: Default> Default for OperationConfig<B> {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: B::default(),
        }
    }
}

impl OperationConfig<AccessControlBackend> {
    pub fn is_lsm(&self) -> bool {
        self.enabled && self.backend == AccessControlBackend::Lsm
    }
    pub fn is_seccomp(&self) -> bool {
        self.enabled && self.backend == AccessControlBackend::Seccomp
    }
}

impl OperationConfig<FileOpenBackend> {
    pub fn is_lsm(&self) -> bool {
        self.enabled && self.backend == FileOpenBackend::Lsm
    }
    pub fn is_fanotify(&self) -> bool {
        self.enabled && self.backend == FileOpenBackend::Fanotify
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileControlConfig {
    pub open: OperationConfig<FileOpenBackend>,
    pub mkdir: OperationConfig<AccessControlBackend>,
    pub rmdir: OperationConfig<AccessControlBackend>,
}

impl Default for FileControlConfig {
    fn default() -> Self {
        Self {
            open: OperationConfig::default(),
            mkdir: OperationConfig {
                enabled: false,
                ..Default::default()
            },
            rmdir: OperationConfig {
                enabled: false,
                ..Default::default()
            },
        }
    }
}

impl FileControlConfig {
    pub fn enabled(&self) -> bool {
        self.open.enabled || self.mkdir.enabled || self.rmdir.enabled
    }
    pub fn uses_lsm(&self) -> bool {
        self.open.is_lsm() || self.mkdir.is_lsm() || self.rmdir.is_lsm()
    }
    pub fn seccomp_syscalls(&self) -> Vec<EnforcementSeccompSyscall> {
        let mut syscalls = Vec::new();
        if self.mkdir.is_seccomp() {
            syscalls.push(EnforcementSeccompSyscall::Mkdir);
        }
        if self.rmdir.is_seccomp() {
            syscalls.push(EnforcementSeccompSyscall::Rmdir);
        }
        syscalls
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CommandRoutingConfig {
    pub exec: OperationConfig<AccessControlBackend>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkRoutingConfig {
    pub connect: OperationConfig<AccessControlBackend>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccessControlBackends {
    pub lsm: LsmConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccessControlConfig {
    pub file: FileControlConfig,
    pub command: CommandRoutingConfig,
    pub network: NetworkRoutingConfig,
    pub backends: AccessControlBackends,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LsmDenySignal {
    #[default]
    None,
    Stop,
    Kill,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LsmConfig {
    /// Maximum entries in each complete published policy, including defaults (default: 4096).
    pub policy_max_entries: u32,
    /// Maximum path buffer bytes including the terminator; bounded by the ABI (default: 252).
    pub path_max_bytes: u32,
    /// Ring buffer capacity in bytes; power of two, at least 65536 (default: 1048576).
    pub audit_buffer_bytes: u32,
    /// Maximum audit events drained per daemon cycle (default: 1024).
    pub audit_batch_max: u32,
    /// Include decisions allowed by the default policy in audit output (default: false).
    pub audit_default_allow: bool,
    /// Boundary alert aggregation window in milliseconds; positive (default: 60000).
    pub alert_window_ms: u32,
    /// Maximum newly admitted boundary alert groups per window; positive (default: 256).
    pub alert_max_groups: u32,
    /// Deny accesses whose resource cannot be resolved (default: true).
    pub deny_on_error: bool,
    /// Signal delivered after a denied access (default: none).
    pub deny_signal: LsmDenySignal,
}

impl Default for LsmConfig {
    fn default() -> Self {
        Self {
            policy_max_entries: 4096,
            path_max_bytes: 252,
            audit_buffer_bytes: 1_048_576,
            audit_batch_max: 1024,
            audit_default_allow: false,
            alert_window_ms: 60_000,
            alert_max_groups: 256,
            deny_on_error: true,
            deny_signal: LsmDenySignal::None,
        }
    }
}

impl AccessControlConfig {
    /// Resolve domain master switches once before assembling runtime services.
    pub fn resolve(&self, file: bool, command: bool, network: bool) -> Self {
        let mut resolved = self.clone();
        resolved.file.open.enabled &= file;
        resolved.file.mkdir.enabled &= file;
        resolved.file.rmdir.enabled &= file;
        resolved.command.exec.enabled &= command;
        resolved.network.connect.enabled &= network;
        resolved
    }

    pub fn uses_lsm(&self) -> bool {
        self.file.uses_lsm() || self.command.exec.is_lsm() || self.network.connect.is_lsm()
    }

    pub fn validate(&self, requests: &[CapabilityRequest]) -> Result<(), String> {
        if self.uses_lsm() {
            let lsm = &self.backends.lsm;
            if lsm.alert_window_ms == 0 || lsm.alert_max_groups == 0 {
                return Err(
                "access_control.backends.lsm requires alert_window_ms > 0 and alert_max_groups > 0"
                    .into(),
            );
            }
            if lsm.policy_max_entries < 5 || lsm.audit_batch_max == 0 {
                return Err("access_control.backends.lsm requires policy_max_entries >= 5 and audit_batch_max > 0".into());
            }
            if !(2..=252).contains(&lsm.path_max_bytes) {
                return Err(
                    "access_control.backends.lsm.path_max_bytes must be between 2 and 252".into(),
                );
            }
            if !lsm.audit_buffer_bytes.is_power_of_two() || lsm.audit_buffer_bytes < 65_536 {
                return Err(
                "access_control.backends.lsm.audit_buffer_bytes must be a power of two >= 65536"
                    .into(),
            );
            }
        }
        for (selected, capability, required) in [
            (
                self.file.open.is_fanotify(),
                Capability::EnforcementFilePermissionFanotify,
                false,
            ),
            (
                self.file.mkdir.is_seccomp(),
                Capability::EnforcementFileMkdirSeccomp,
                true,
            ),
            (
                self.file.rmdir.is_seccomp(),
                Capability::EnforcementFileRmdirSeccomp,
                true,
            ),
            (
                self.file.uses_lsm(),
                Capability::EnforcementFilePermissionLsm,
                true,
            ),
            (
                self.command.exec.is_seccomp(),
                Capability::EnforcementCommandExecutionSeccomp,
                false,
            ),
            (
                self.command.exec.is_lsm(),
                Capability::EnforcementCommandExecutionLsm,
                true,
            ),
            (
                self.network.connect.is_seccomp(),
                Capability::EnforcementNetworkConnectSeccomp,
                false,
            ),
            (
                self.network.connect.is_lsm(),
                Capability::EnforcementNetworkConnectLsm,
                true,
            ),
        ] {
            let requested = requests
                .iter()
                .any(|r| r.capability == capability && r.mode != RequestMode::Disabled);
            if requested && !selected {
                return Err(format!(
                    "{} requires its access_control operation enabled with the matching backend",
                    capability.as_str()
                ));
            }
            if selected
                && required
                && !requests
                    .iter()
                    .any(|r| r.capability == capability && r.mode == RequestMode::Required)
            {
                return Err(format!(
                    "enabled access control requires {} as a required capture capability",
                    capability.as_str()
                ));
            }
        }
        if (self.uses_lsm() || self.file.mkdir.is_seccomp() || self.file.rmdir.is_seccomp())
            && !requests.iter().any(|r| {
                r.capability == Capability::ProcLifecycle && r.mode == RequestMode::Required
            })
        {
            return Err(
                "LSM and directory seccomp control require proc-lifecycle as a required capability"
                    .into(),
            );
        }
        Ok(())
    }
}
