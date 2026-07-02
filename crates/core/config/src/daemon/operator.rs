//! Operator-facing config file parsing for daemon and control CLI commands.

#[path = "operator/document.rs"]
mod document;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use storage_factory::StorageConfig;

use super::{
    AgentInvocationConfig, ApplicationProtocolConfig, CommandControlConfig, DiagnosticLogLevel,
    EbpfCollectorConfig, EnforcementConfig, FileObservationConfig, NetworkControlConfig,
    PayloadConfig, PayloadSocketConfig, PayloadTlsConfig, ProcessSeccompConfig,
    ResourceMetricsConfig, RuntimeExportConfig, SeccompNotifyConfig, SemanticRetentionConfig,
    SocketPermissions, SseDataPolicy, TraceFinalizationConfig, WebServerConfig,
    WorkloadDiagnosticsConfig,
};
use crate::capture_profile::{CaptureProfile, LaunchSeccompRequirements};
use crate::export::ExportConfig;
use crate::framework::ConfigModel;
use crate::provider_rules::ProviderRuleSetConfig;

pub const DEFAULT_OPERATOR_CONFIG_PATH: &str = "/etc/actrail/actraild.conf";
pub const DEFAULT_CONTROL_PENDING_CONNECTION_MAX: u32 = 256;
pub const DEFAULT_ACTIVE_TRACE_MAX: u32 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorConfigInitStatus {
    Created,
    ExistingValid,
    Overwritten,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorConfig {
    pub socket_path: PathBuf,
    pub socket_permissions: SocketPermissions,
    pub control_pending_connection_max: u32,
    pub active_trace_max: u32,
    pub pid_file: PathBuf,
    pub storage: StorageConfig,
    pub web: WebServerConfig,
    pub export_config: ExportConfig,
    pub export_runtime: RuntimeExportConfig,
    pub startup_plugins: StartupPluginsConfig,
    pub log_path: PathBuf,
    pub diagnostic_log_level: DiagnosticLogLevel,
    pub workload_diagnostics: WorkloadDiagnosticsConfig,
    pub capture_profile: CaptureProfile,
    pub ebpf_config: EbpfCollectorConfig,
    pub payload_config: PayloadConfig,
    pub seccomp_notify: SeccompNotifyConfig,
    pub process_seccomp: ProcessSeccompConfig,
    pub agent_invocation: AgentInvocationConfig,
    pub semantic_retention: SemanticRetentionConfig,
    pub file_observation: FileObservationConfig,
    pub application_protocol: ApplicationProtocolConfig,
    pub resource_metrics: ResourceMetricsConfig,
    pub trace_finalization: TraceFinalizationConfig,
    pub provider_rule_set: Option<ProviderRuleSetConfig>,
    pub enforcement: EnforcementConfig,
    pub command_control: CommandControlConfig,
    pub network_control: NetworkControlConfig,
    pub startup_wait_ms: u64,
    pub shutdown_wait_ms: u64,
    pub supervision_poll_interval_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupPluginFailurePolicy {
    FailFast,
    Continue,
}

impl StartupPluginFailurePolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FailFast => "fail-fast",
            Self::Continue => "continue",
        }
    }
}

impl FromStr for StartupPluginFailurePolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "fail-fast" => Ok(Self::FailFast),
            "continue" => Ok(Self::Continue),
            _ => Err("expected fail-fast or continue".to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPluginsConfig {
    pub enabled: bool,
    pub failure_policy: StartupPluginFailurePolicy,
    pub load: Vec<StartupPluginLoadConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPluginLoadConfig {
    pub instance_id: String,
    pub enabled: bool,
    pub failure_policy: Option<StartupPluginFailurePolicy>,
    pub manifest_path: PathBuf,
    pub plugin_config_path: Option<PathBuf>,
    pub host_grants: Vec<String>,
}

/// Derive the seccomp-notify capabilities a launch must install from the
/// payload, process, and network config. A payload backend needs the notify
/// path only when it is enabled *and* its capture backend cannot collect
/// without it. Single source of truth shared by ctl and the daemon.
pub fn launch_seccomp_requirements(
    payload: &PayloadConfig,
    process_seccomp: &ProcessSeccompConfig,
    network_control: &NetworkControlConfig,
) -> LaunchSeccompRequirements {
    LaunchSeccompRequirements::new(
        payload.tls.enabled && payload.tls.capture_backend.requires_seccomp_notify(),
        payload.socket.enabled && payload.socket.capture_backend.requires_seccomp_notify(),
        process_seccomp.enabled,
        network_control.enabled,
    )
}

impl OperatorConfig {
    /// The seccomp-notify capabilities this config asks a launch to install.
    /// Delegates to [`launch_seccomp_requirements`] so ctl and the daemon
    /// derive identical requirements from one place.
    pub fn launch_seccomp_requirements(
        &self,
    ) -> crate::capture_profile::LaunchSeccompRequirements {
        launch_seccomp_requirements(
            &self.payload_config,
            &self.process_seccomp,
            &self.network_control,
        )
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("read config {}: {error}", path.display()))?;
        Self::parse(&raw)
    }

    pub fn init() -> Result<Self, String> {
        let default_config = Self::default_hierarchical_template()?;
        Self::parse(&default_config)
            .map_err(|error| format!("validate default operator config: {error}"))
    }

    pub fn patch_file(&self, patch_path: &Path) -> Result<Self, String> {
        let patch = fs::read_to_string(patch_path)
            .map_err(|error| format!("read config patch {}: {error}", patch_path.display()))?;
        self.patch(&patch)
    }

    pub fn patch(&self, patch: &str) -> Result<Self, String> {
        let base = self.dump()?;
        let mut base_value: toml::Value = toml::from_str(&base)
            .map_err(|error| format!("parse base operator config: {error}"))?;
        let patch_value: toml::Value = toml::from_str(patch)
            .map_err(|error| format!("parse operator config patch: {error}"))?;
        merge_toml_value(&mut base_value, patch_value);
        let merged = toml::to_string_pretty(&base_value)
            .map_err(|error| format!("render patched operator config: {error}"))?;
        Self::parse(&merged).map_err(|error| format!("validate patched operator config: {error}"))
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        document::OperatorDocument::parse(raw)?.to_config()
    }

    pub fn default_hierarchical_template() -> Result<String, String> {
        document::OperatorDocument::default_toml()
    }

    pub fn to_hierarchical_toml(&self) -> Result<String, String> {
        document::OperatorDocument::from_config(self)
            .to_toml()
            .map_err(|error| error.to_string())
    }

    pub fn dump(&self) -> Result<String, String> {
        self.to_hierarchical_toml()
    }

    pub fn dump_to_path(&self, path: &Path, overwrite: bool) -> Result<(), String> {
        let raw = self.dump()?;
        let mode = if overwrite {
            WriteMode::Overwrite
        } else {
            WriteMode::CreateNew
        };
        write_operator_config(path, mode, &raw)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriteMode {
    CreateNew,
    Overwrite,
}

fn write_operator_config(path: &Path, mode: WriteMode, raw: &str) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create config directory {}: {error}", parent.display()))?;
    }
    let mut options = OpenOptions::new();
    options.write(true);
    match mode {
        WriteMode::CreateNew => {
            options.create_new(true);
        }
        WriteMode::Overwrite => {
            options.create(true).truncate(true);
        }
    }
    let action = match mode {
        WriteMode::CreateNew => "create",
        WriteMode::Overwrite => "overwrite",
    };
    let mut file = options
        .open(path)
        .map_err(|error| format!("{action} config {}: {error}", path.display()))?;
    file.write_all(raw.as_bytes())
        .map_err(|error| format!("write config {}: {error}", path.display()))
}

fn merge_toml_value(base: &mut toml::Value, patch: toml::Value) {
    match (base, patch) {
        (toml::Value::Table(base), toml::Value::Table(patch)) => {
            for (key, patch_value) in patch {
                match base.get_mut(&key) {
                    Some(base_value)
                        if matches!(base_value, toml::Value::Table(_))
                            && matches!(patch_value, toml::Value::Table(_)) =>
                    {
                        merge_toml_value(base_value, patch_value);
                    }
                    _ => {
                        base.insert(key, patch_value);
                    }
                }
            }
        }
        (base, patch) => {
            *base = patch;
        }
    }
}

fn validate_seccomp_config(
    notify: &SeccompNotifyConfig,
    payload_tls: &PayloadTlsConfig,
    payload_socket: &PayloadSocketConfig,
    process_seccomp: &ProcessSeccompConfig,
    capabilities: &[CapabilityRequest],
) -> Result<(), String> {
    if payload_tls.enabled
        && payload_tls.capture_backend.requires_seccomp_notify()
        && !notify.enabled
    {
        return Err("payload_tls capture backend requires seccomp_notify_enabled=true".to_string());
    }
    if payload_socket.enabled
        && payload_socket.capture_backend.requires_seccomp_notify()
        && !notify.enabled
    {
        return Err(
            "payload_socket capture backend requires seccomp_notify_enabled=true".to_string(),
        );
    }
    if process_seccomp.enabled && !notify.enabled {
        return Err(
            "process_seccomp_enabled=true requires seccomp_notify_enabled=true".to_string(),
        );
    }
    if capability_requested(capabilities, &Capability::ProcExecContext) && !process_seccomp.enabled
    {
        return Err("proc-exec-context requires process_seccomp_enabled=true".to_string());
    }
    Ok(())
}

fn validate_application_protocol_config(
    config: &ApplicationProtocolConfig,
    payload_tls_enabled: bool,
    payload_socket_enabled: bool,
    capabilities: &[CapabilityRequest],
) -> Result<(), String> {
    if config.http1_enabled && !config.enabled {
        return Err(
            "application_protocol_http1_enabled requires application_protocol_enabled=true"
                .to_string(),
        );
    }
    if config.http2_enabled && !config.enabled {
        return Err(
            "application_protocol_http2_enabled requires application_protocol_enabled=true"
                .to_string(),
        );
    }
    if config.sse_enabled && !config.http1_enabled {
        return Err(
            "application_http_sse_enabled requires application_protocol_http1_enabled=true"
                .to_string(),
        );
    }
    if matches!(config.sse_data_policy, SseDataPolicy::Preview) && !config.sse_enabled {
        return Err(
            "application_http_sse_data_policy=preview requires application_http_sse_enabled=true"
                .to_string(),
        );
    }
    let http1_requested =
        capability_requested(capabilities, &Capability::NetApplicationPlaintextHttp);
    if http1_requested && !(config.enabled && config.http1_enabled) {
        return Err(
            "net-application-plaintext-http requires application_protocol_enabled=true and application_protocol_http1_enabled=true"
                .to_string(),
        );
    }
    let tls_payload_requested =
        capability_requested(capabilities, &Capability::TlsPlaintextPayload);
    let socket_payload_requested =
        capability_requested(capabilities, &Capability::SocketPlaintextPayload);
    let plaintext_payload_available = (payload_tls_enabled && tls_payload_requested)
        || (payload_socket_enabled && socket_payload_requested);
    if http1_requested && !plaintext_payload_available {
        return Err(
            "net-application-plaintext-http requires enabled tls-plaintext-payload or socket-plaintext-payload in the same profile"
                .to_string(),
        );
    }
    let http2_requested =
        capability_requested(capabilities, &Capability::NetApplicationHttp2Frames);
    if http2_requested && !(config.enabled && config.http2_enabled) {
        return Err(
            "net-application-http2-frames requires application_protocol_enabled=true and application_protocol_http2_enabled=true"
                .to_string(),
        );
    }
    if http2_requested && !plaintext_payload_available {
        return Err(
            "net-application-http2-frames requires enabled tls-plaintext-payload or socket-plaintext-payload in the same profile"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_resource_metrics_config(
    config: &ResourceMetricsConfig,
    capabilities: &[CapabilityRequest],
) -> Result<(), String> {
    if capability_requested(capabilities, &Capability::ResourceMetrics) && !config.enabled {
        return Err("resource-metrics requires resource_metrics_enabled=true".to_string());
    }
    Ok(())
}

fn validate_enforcement_config(
    config: &EnforcementConfig,
    capabilities: &[CapabilityRequest],
) -> Result<(), String> {
    if capability_requested(capabilities, &Capability::EnforcementFilePermissionFanotify)
        && !config.enabled
    {
        return Err(
            "enforcement-file-permission-fanotify requires enforcement_enabled=true".to_string(),
        );
    }
    Ok(())
}

fn capability_requested(capabilities: &[CapabilityRequest], capability: &Capability) -> bool {
    capabilities
        .iter()
        .any(|request| request.mode != RequestMode::Disabled && request.capability == *capability)
}
