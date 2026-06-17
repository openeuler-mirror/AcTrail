//! Operator-facing config file parsing for daemon and control CLI commands.

#[path = "operator/sections.rs"]
mod sections;
#[path = "operator/template.rs"]
mod template;

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::ProfileName;
use storage_factory::StorageConfig;

use super::values::ConfigValues;
use super::{
    AgentInvocationConfig, ApplicationProtocolConfig, DiagnosticLogLevel, EbpfCollectorConfig,
    EnforcementConfig, FileObservationConfig, PayloadConfig, PayloadSocketConfig, PayloadTlsConfig,
    ProcessSeccompConfig, ResourceMetricsConfig, RuntimeExportConfig, SeccompNotifyConfig,
    SemanticRetentionConfig, SocketPermissions, SseDataPolicy,
};
use crate::capture_profile::CaptureProfile;
use crate::export::ExportConfig;
use crate::provider_rules::ProviderRuleSetConfig;

pub const DEFAULT_OPERATOR_CONFIG_PATH: &str = "/etc/actrail/actraild.conf";
pub const DEFAULT_CONTROL_PENDING_CONNECTION_MAX: u32 = 256;
pub use template::OPERATOR_CONFIG_TEMPLATE;

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
    pub pid_file: PathBuf,
    pub storage: StorageConfig,
    pub export_config: ExportConfig,
    pub export_runtime: RuntimeExportConfig,
    pub log_path: PathBuf,
    pub diagnostic_log_level: DiagnosticLogLevel,
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
    pub provider_rule_set: Option<ProviderRuleSetConfig>,
    pub enforcement: EnforcementConfig,
    pub startup_wait_ms: u64,
    pub shutdown_wait_ms: u64,
    pub supervision_poll_interval_ms: u64,
}

impl OperatorConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("read config {}: {error}", path.display()))?;
        Self::parse(&raw)
    }

    pub fn initialize(path: &Path, force: bool) -> Result<OperatorConfigInitStatus, String> {
        if force {
            let existed = match fs::symlink_metadata(path) {
                Ok(_) => true,
                Err(error) if error.kind() == ErrorKind::NotFound => false,
                Err(error) => return Err(format!("inspect config {}: {error}", path.display())),
            };
            Self::parse(OPERATOR_CONFIG_TEMPLATE)
                .map_err(|error| format!("validate default operator config: {error}"))?;
            write_default_operator_config(path, WriteMode::Overwrite)?;
            return Ok(if existed {
                OperatorConfigInitStatus::Overwritten
            } else {
                OperatorConfigInitStatus::Created
            });
        }
        match fs::read_to_string(path) {
            Ok(raw) => {
                Self::parse(&raw)
                    .map_err(|error| format!("validate config {}: {error}", path.display()))?;
                Ok(OperatorConfigInitStatus::ExistingValid)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Self::parse(OPERATOR_CONFIG_TEMPLATE)
                    .map_err(|error| format!("validate default operator config: {error}"))?;
                write_default_operator_config(path, WriteMode::CreateNew)?;
                Ok(OperatorConfigInitStatus::Created)
            }
            Err(error) => Err(format!("read config {}: {error}", path.display())),
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let values = ConfigValues::parse(raw)?;
        let profile_name = ProfileName::new(values.required("profile_name")?);
        let capabilities = values.capability_requests()?;
        if capabilities.is_empty() {
            return Err("at least one capability is required".to_string());
        }
        let diagnostic_log_level = values
            .required("diagnostic_log_level")?
            .parse::<DiagnosticLogLevel>()
            .map_err(|error| format!("invalid diagnostic_log_level: {error}"))?;
        let payload_tls = sections::payload_tls_config(values.node("payload_tls"))?;
        let payload_stdio = sections::payload_stdio_config(values.node("payload_stdio"))?;
        let payload_socket = sections::payload_socket_config(values.node("payload_socket"))?;
        let payload_config = PayloadConfig {
            tls: payload_tls,
            stdio: payload_stdio,
            socket: payload_socket,
        };
        let seccomp_notify = sections::seccomp_notify_config(values.node("seccomp_notify"))?;
        let process_seccomp = sections::process_seccomp_config(values.node("process_seccomp"))?;
        let agent_invocation = sections::agent_invocation_config(values.node("agent_invocation"))?;
        let semantic_retention =
            sections::semantic_retention_config(values.node("semantic_retention"))?;
        let file_observation = sections::file_observation_config(values.node("file_observation"))?;
        let application_protocol = sections::application_protocol_config(
            values.node("application_protocol"),
            values.node("application_http"),
            values.node("application_http2"),
        )?;
        validate_application_protocol_config(
            &application_protocol,
            payload_config.tls.enabled,
            payload_config.socket.enabled,
            &capabilities,
        )?;
        let resource_metrics = sections::resource_metrics_config(values.node("resource_metrics"))?;
        validate_resource_metrics_config(&resource_metrics, &capabilities)?;
        let enforcement = sections::enforcement_config(values.node("enforcement"))?;
        validate_enforcement_config(&enforcement, &capabilities)?;
        validate_seccomp_config(
            &seccomp_notify,
            &payload_config.tls,
            &payload_config.socket,
            &process_seccomp,
            &capabilities,
        )?;
        let export_config = sections::export_config(&values, values.node("export"))?;
        let export_runtime = RuntimeExportConfig::parse(raw)?;
        let provider_rule_set = sections::provider_rule_set_config(values.node("provider"))?;
        let storage = StorageConfig::parse(raw)?;
        Ok(Self {
            socket_path: PathBuf::from(values.required("socket_path")?),
            socket_permissions: SocketPermissions {
                mode: values.required_octal("socket_mode_octal")?,
            },
            control_pending_connection_max: values.optional_positive_u32(
                "control_pending_connection_max",
                DEFAULT_CONTROL_PENDING_CONNECTION_MAX,
            )?,
            pid_file: PathBuf::from(values.required("pid_file")?),
            storage,
            export_config,
            export_runtime,
            log_path: PathBuf::from(values.required("log_path")?),
            diagnostic_log_level,
            capture_profile: CaptureProfile::new(profile_name, capabilities),
            ebpf_config: EbpfCollectorConfig {
                enabled: values.required_bool("ebpf_enabled")?,
                memlock_rlimit: values.required_memlock_rlimit("memlock_rlimit")?,
                tracked_process_max_entries: values.required_u32("tracked_process_max_entries")?,
                pending_operation_max_entries: values
                    .required_u32("pending_operation_max_entries")?,
                suppressed_fd_max_entries: values.required_u32("suppressed_fd_max_entries")?,
                event_ring_buffer_max_bytes: values.required_u32("event_ring_buffer_max_bytes")?,
                file_path_capture_enabled: values.required_bool("file_path_capture_enabled")?,
                file_path_max_bytes: values.required_positive_u32("file_path_max_bytes")?,
            },
            payload_config,
            seccomp_notify,
            process_seccomp,
            agent_invocation,
            semantic_retention,
            file_observation,
            application_protocol,
            resource_metrics,
            provider_rule_set,
            enforcement,
            startup_wait_ms: values.required_positive_u64("startup_wait_ms")?,
            shutdown_wait_ms: values.required_positive_u64("shutdown_wait_ms")?,
            supervision_poll_interval_ms: values
                .required_positive_u64("supervision_poll_interval_ms")?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriteMode {
    CreateNew,
    Overwrite,
}

fn write_default_operator_config(path: &Path, mode: WriteMode) -> Result<(), String> {
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
    file.write_all(OPERATOR_CONFIG_TEMPLATE.as_bytes())
        .map_err(|error| format!("write config {}: {error}", path.display()))
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

#[cfg(test)]
#[path = "operator/tests.rs"]
mod tests;
