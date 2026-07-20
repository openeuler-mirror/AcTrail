//! Hierarchical operator config document.

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use export_factory::{
    ExportConfig as RuntimeExportConfig, ExportDeliveryConfig, ExportRouteConfig, ExportRouteKind,
    ExportRouteTargetConfig, OtelJsonlExporterConfig,
};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::ProfileName;
use serde::{Deserialize, Serialize};
use storage_factory::StorageConfig;

use super::super::{
    AgentInvocationConfig, ApplicationProtocolConfig, ClusterCenterConfig, ClusterConfig,
    ClusterReportConfig, CommandControlConfig, DEFAULT_ACTIVE_TRACE_MAX,
    DEFAULT_CONTROL_PENDING_CONNECTION_MAX, DEFAULT_FINALIZATION_POLL_INTERVAL_MS,
    DEFAULT_FINALIZATION_TRACES_PER_CYCLE, DisabledOrPath, EbpfCollectorConfig, EbpfEnabledMode,
    EnforcementBackend, EnforcementBuiltinRuleConfig, EnforcementConfig, EnforcementMarkStrategy,
    EnforcementScope, FileBulkReadFastPathConfig, FileBulkReadObservationConfig,
    FileMetadataRetention, FileObservationConfig, FileRawEventRetention, FileTtyObservationConfig,
    FsEnumerateObservationConfig, Http2DataContentRetention, HttpBodyRetention,
    HttpHeadersRetention, L0LlmCallRetention, L1SseRetention, L2HttpRetention,
    L3Http2FrameRetention, L4PayloadRetention, LlmRequestContentRetention,
    LlmResponseContentRetention, LlmToolCallRetention, LlmUsageRetention, MemlockRlimit,
    NetworkControlConfig, NetworkControlSeccompSyscall, PayloadBodyContentRetention, PayloadConfig,
    PayloadRedactionPolicy, PayloadSocketCaptureBackend, PayloadSocketConfig,
    PayloadSocketSeccompSyscall, PayloadStdioConfig, PayloadStdioStorageMode,
    PayloadTlsCaptureBackend, PayloadTlsConfig, PayloadTlsLibrary, PayloadTlsLibraryPath,
    PayloadTlsResolver, PayloadTlsSeccompSyscall, PayloadTlsSource,
    PayloadTlsSyncRuntimeLibraryPath, ProcessSeccompConfig, ProcessSeccompSyscall,
    ResourceMetricsConfig, SeccompNotifyConfig, SemanticContentOwner, SemanticRetentionConfig,
    SocketPermissions, SseDataPolicy, SseEventContentRetention, StartupPluginFailurePolicy,
    StartupPluginLoadConfig, StartupPluginsConfig, StorageRetentionConfig, TraceFinalizationConfig,
    WebServerConfig, WorkloadDiagnosticsConfig,
};
use super::{
    OperatorConfig, validate_application_protocol_config, validate_enforcement_config,
    validate_resource_metrics_config, validate_seccomp_config,
};
use crate::capture_profile::CaptureProfile;
use crate::export::ExportConfig;
use crate::framework::{ConfigError, ConfigModel};
use crate::provider_rules::ProviderRuleSetConfig;

#[path = "document/app.rs"]
mod app;
#[path = "document/base.rs"]
mod base;
#[path = "document/cluster.rs"]
mod cluster;
#[path = "document/command.rs"]
mod command;
#[path = "document/file.rs"]
mod file;
#[path = "document/helpers.rs"]
mod helpers;
#[path = "document/network.rs"]
mod network;
#[path = "document/payload.rs"]
mod payload;
#[path = "document/process.rs"]
mod process;
#[path = "document/semantic.rs"]
mod semantic;

use app::*;
use base::*;
use cluster::*;
use command::*;
use file::*;
use helpers::*;
use network::*;
use payload::*;
use process::*;
use semantic::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct OperatorDocument {
    control: ControlDocument,
    storage: StorageDocument,
    web: WebDocument,
    cluster: ClusterDocument,
    export: ExportDocument,
    plugins: PluginsDocument,
    capture: CaptureDocument,
    ebpf: EbpfDocument,
    payload: PayloadDocument,
    seccomp_notify: SeccompNotifyDocument,
    process_seccomp: ProcessSeccompDocument,
    agent_invocation: AgentInvocationDocument,
    semantic_retention: SemanticRetentionDocument,
    file_observation: FileObservationDocument,
    application: ApplicationDocument,
    resource_metrics: ResourceMetricsDocument,
    provider: ProviderDocument,
    enforcement: EnforcementDocument,
    command_control: CommandControlDocument,
    network_control: NetworkControlDocument,
    supervision: SupervisionDocument,
}

impl Default for OperatorDocument {
    fn default() -> Self {
        Self {
            control: ControlDocument::default(),
            storage: StorageDocument::default(),
            web: WebDocument::default(),
            cluster: ClusterDocument::default(),
            export: ExportDocument::default(),
            plugins: PluginsDocument::default(),
            capture: CaptureDocument::default(),
            ebpf: EbpfDocument::default(),
            payload: PayloadDocument::default(),
            seccomp_notify: SeccompNotifyDocument::default(),
            process_seccomp: ProcessSeccompDocument::default(),
            agent_invocation: AgentInvocationDocument::default(),
            semantic_retention: SemanticRetentionDocument::default(),
            file_observation: FileObservationDocument::default(),
            application: ApplicationDocument::default(),
            resource_metrics: ResourceMetricsDocument::default(),
            provider: ProviderDocument::default(),
            enforcement: EnforcementDocument::default(),
            command_control: CommandControlDocument::default(),
            network_control: NetworkControlDocument::default(),
            supervision: SupervisionDocument::default(),
        }
    }
}

impl ConfigModel for OperatorDocument {
    const MODEL_NAME: &'static str = "operator";

    fn validate(&self) -> Result<(), ConfigError> {
        if self.capture.capabilities.is_empty() {
            return Err(ConfigError::new("capture.capabilities must not be empty"));
        }
        if self.export.runtime.enabled
            && self
                .export
                .runtime
                .routes
                .iter()
                .all(|route| !route.enabled)
        {
            return Err(ConfigError::new(
                "export.runtime.enabled=true requires at least one enabled route",
            ));
        }
        Ok(())
    }
}

impl OperatorDocument {
    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        Self::from_toml(raw).map_err(|error| error.to_string())
    }

    pub(super) fn default_toml() -> Result<String, String> {
        Self::default().to_toml().map_err(|error| error.to_string())
    }

    pub(super) fn from_config(config: &OperatorConfig) -> Self {
        let mut required = Vec::new();
        let mut opportunistic = Vec::new();
        let mut disabled = Vec::new();
        for request in &config.capture_profile.capabilities {
            let target = match request.mode {
                RequestMode::Required => &mut required,
                RequestMode::Opportunistic => &mut opportunistic,
                RequestMode::Disabled => &mut disabled,
            };
            target.push(capability_as_str(&request.capability).to_string());
        }
        let storage = StorageDocument {
            backend: config.storage.backend().as_str().to_string(),
            sqlite: SqliteStorageDocument {
                path: config.storage.path().display().to_string(),
                busy_timeout_ms: config.storage.sqlite_busy_timeout_ms(),
            },
            retention: StorageRetentionDocument::from_config(&config.storage_retention),
        };
        Self {
            control: ControlDocument {
                socket_path: config.socket_path.display().to_string(),
                socket_mode_octal: format!("{:o}", config.socket_permissions.mode),
                pending_connection_max: config.control_pending_connection_max,
                active_trace_max: config.active_trace_max,
                pid_file: config.pid_file.display().to_string(),
                log_path: config.log_path.display().to_string(),
                diagnostic_log_level: diagnostic_log_level_as_str(config.diagnostic_log_level)
                    .to_string(),
                workload_diagnostics: WorkloadDiagnosticsDocument {
                    enabled: config.workload_diagnostics.enabled,
                    interval_ms: config.workload_diagnostics.interval_ms,
                },
                finalization: FinalizationDocument {
                    traces_per_cycle: config.trace_finalization.traces_per_cycle,
                    poll_interval_ms: config.trace_finalization.poll_interval_ms,
                },
            },
            storage,
            web: WebDocument {
                listen_addr: config.web.listen_addr.to_string(),
                request_read_timeout_ms: config
                    .web
                    .request_read_timeout
                    .map(|duration| duration.as_millis().to_string())
                    .unwrap_or_else(|| "disabled".to_string()),
            },
            cluster: ClusterDocument::from_config(&config.cluster),
            export: ExportDocument {
                snapshot: SnapshotExportDocument {
                    graph_schema_version: config.export_config.graph_schema_version.clone(),
                    allow_active_trace_snapshot: config.export_config.allow_active_trace_snapshot,
                    directory: config.export_config.output_directory.display().to_string(),
                    payload_bytes_enabled: config.export_config.payload_bytes_enabled,
                    payload_text_enabled: config.export_config.payload_text_enabled,
                },
                runtime: RuntimeExportDocument::from_config(&config.export_runtime),
            },
            plugins: PluginsDocument::from_config(&config.startup_plugins),
            capture: CaptureDocument {
                profile_name: config.capture_profile.name.as_str().to_string(),
                capabilities: required,
                opportunistic_capabilities: opportunistic,
                disabled_capabilities: disabled,
            },
            ebpf: EbpfDocument {
                enabled: config.ebpf_config.enabled_mode.to_string(),
                memlock_rlimit: memlock_rlimit_as_str(config.ebpf_config.memlock_rlimit),
                tracked_process_max_entries: config.ebpf_config.tracked_process_max_entries,
                pending_operation_max_entries: config.ebpf_config.pending_operation_max_entries,
                suppressed_fd_max_entries: config.ebpf_config.suppressed_fd_max_entries,
                suppressed_fd_index_slots_per_process: config
                    .ebpf_config
                    .suppressed_fd_index_slots_per_process,
                event_ring_buffer_max_bytes: config.ebpf_config.event_ring_buffer_max_bytes,
                file_path_capture_enabled: config.ebpf_config.file_path_capture_enabled,
                file_path_max_bytes: config.ebpf_config.file_path_max_bytes,
            },
            payload: PayloadDocument::from_config(&config.payload_config),
            seccomp_notify: SeccompNotifyDocument {
                enabled: config.seccomp_notify.enabled,
                reserved_listener_fd: config.seccomp_notify.reserved_listener_fd,
            },
            process_seccomp: ProcessSeccompDocument {
                enabled: config.process_seccomp.enabled,
                syscalls: config
                    .process_seccomp
                    .syscalls
                    .iter()
                    .map(process_seccomp_syscall_as_str)
                    .map(str::to_string)
                    .collect(),
                max_args: config.process_seccomp.max_args,
                max_arg_bytes: config.process_seccomp.max_arg_bytes,
                pending_max_entries: config.process_seccomp.pending_max_entries,
            },
            agent_invocation: AgentInvocationDocument {
                enabled: config.agent_invocation.enabled,
                commands: config.agent_invocation.commands.clone(),
            },
            semantic_retention: SemanticRetentionDocument::from_config(&config.semantic_retention),
            file_observation: FileObservationDocument::from_config(&config.file_observation),
            application: ApplicationDocument::from_config(&config.application_protocol),
            resource_metrics: ResourceMetricsDocument {
                enabled: config.resource_metrics.enabled,
                interval_ms: config.resource_metrics.interval_ms,
                include_children: config.resource_metrics.include_children,
                include_system: config.resource_metrics.include_system,
                cpu_alert_percent_millis: disabled_or_u64_as_string(
                    config.resource_metrics.cpu_alert_percent_millis,
                ),
                memory_alert_rss_kb: disabled_or_u64_as_string(
                    config.resource_metrics.memory_alert_rss_kb,
                ),
            },
            provider: ProviderDocument {
                rules_enabled: config.provider_rule_set.is_some(),
                rules_path: config
                    .provider_rule_set
                    .as_ref()
                    .map(|provider| provider.rules_path.display().to_string())
                    .unwrap_or_else(|| ProviderDocument::default().rules_path),
                unknown_provider_label: config
                    .provider_rule_set
                    .as_ref()
                    .map(|provider| provider.unknown_provider_label.clone())
                    .unwrap_or_else(|| ProviderDocument::default().unknown_provider_label),
            },
            enforcement: EnforcementDocument {
                enabled: config.enforcement.enabled,
                backend: enforcement_backend_as_str(config.enforcement.backend).to_string(),
                scope: enforcement_scope_as_str(config.enforcement.scope).to_string(),
                rules_path: config.enforcement.rules_path.display().to_string(),
                builtin_rules: config
                    .enforcement
                    .builtin_rules
                    .iter()
                    .map(EnforcementBuiltinRuleDocument::from_config)
                    .collect(),
                default_decision: config.enforcement.default_decision.as_str().to_string(),
                mark_strategy: enforcement_mark_strategy_as_str(config.enforcement.mark_strategy)
                    .to_string(),
                audit_enabled: config.enforcement.audit_enabled,
                event_buffer_bytes: config.enforcement.event_buffer_bytes,
            },
            command_control: CommandControlDocument {
                enabled: config.command_control.enabled,
                rules_path: config.command_control.rules_path.display().to_string(),
            },
            network_control: NetworkControlDocument {
                enabled: config.network_control.enabled,
                rules_path: config.network_control.rules_path.display().to_string(),
                syscalls: config
                    .network_control
                    .syscalls
                    .iter()
                    .copied()
                    .map(network_control_seccomp_syscall_as_str)
                    .map(str::to_string)
                    .collect(),
            },
            supervision: SupervisionDocument {
                startup_wait_ms: config.startup_wait_ms,
                shutdown_wait_ms: config.shutdown_wait_ms,
                poll_interval_ms: config.supervision_poll_interval_ms,
            },
        }
    }

    pub(super) fn to_config(&self) -> Result<OperatorConfig, String> {
        let capabilities = self.capture.capability_requests()?;
        if capabilities.is_empty() {
            return Err("at least one capability is required".to_string());
        }
        let payload_config = PayloadConfig {
            tls: self.payload.tls.to_config()?,
            stdio: self.payload.stdio.to_config()?,
            socket: self.payload.socket.to_config()?,
        };
        let seccomp_notify = self.seccomp_notify.to_config();
        let process_seccomp = self.process_seccomp.to_config()?;
        let application_protocol = self.application.to_config()?;
        validate_application_protocol_config(
            &application_protocol,
            payload_config.tls.enabled,
            payload_config.socket.enabled,
            &capabilities,
        )?;
        let resource_metrics = self.resource_metrics.to_config()?;
        validate_resource_metrics_config(&resource_metrics, &capabilities)?;
        let enforcement = self.enforcement.to_config()?;
        validate_enforcement_config(&enforcement, &capabilities)?;
        let command_control = self.command_control.to_config();
        let network_control = self.network_control.to_config()?;
        validate_seccomp_config(
            &seccomp_notify,
            &payload_config.tls,
            &payload_config.socket,
            &process_seccomp,
            &capabilities,
        )?;
        Ok(OperatorConfig {
            socket_path: PathBuf::from(&self.control.socket_path),
            socket_permissions: SocketPermissions {
                mode: parse_octal("control.socket_mode_octal", &self.control.socket_mode_octal)?,
            },
            control_pending_connection_max: require_positive_u32(
                "control.pending_connection_max",
                self.control.pending_connection_max,
            )?,
            active_trace_max: require_positive_u32(
                "control.active_trace_max",
                self.control.active_trace_max,
            )?,
            pid_file: PathBuf::from(&self.control.pid_file),
            storage: self.storage.to_config()?,
            storage_retention: self.storage.retention.to_config()?,
            web: self.web.to_config()?,
            cluster: self.cluster.to_config()?,
            export_config: self.export.snapshot.to_config(),
            export_runtime: self.export.runtime.to_config()?,
            startup_plugins: self.plugins.to_config()?,
            log_path: PathBuf::from(&self.control.log_path),
            diagnostic_log_level: parse_value(
                "control.diagnostic_log_level",
                &self.control.diagnostic_log_level,
            )?,
            workload_diagnostics: self.control.workload_diagnostics.to_config()?,
            capture_profile: CaptureProfile::new(
                ProfileName::new(self.capture.profile_name.clone()),
                capabilities,
            ),
            ebpf_config: self.ebpf.to_config()?,
            payload_config,
            seccomp_notify,
            process_seccomp,
            agent_invocation: self.agent_invocation.to_config(),
            semantic_retention: self.semantic_retention.to_config()?,
            file_observation: self.file_observation.to_config()?,
            application_protocol,
            resource_metrics,
            trace_finalization: self.control.finalization.to_config()?,
            provider_rule_set: self.provider.to_config(),
            enforcement,
            command_control,
            network_control,
            startup_wait_ms: require_positive_u64(
                "supervision.startup_wait_ms",
                self.supervision.startup_wait_ms,
            )?,
            shutdown_wait_ms: require_positive_u64(
                "supervision.shutdown_wait_ms",
                self.supervision.shutdown_wait_ms,
            )?,
            supervision_poll_interval_ms: require_positive_u64(
                "supervision.poll_interval_ms",
                self.supervision.poll_interval_ms,
            )?,
        })
    }
}
