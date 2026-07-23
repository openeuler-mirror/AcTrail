//! Construction for the storage-backed attach service.

use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, CommandControlConfig, DiagnosticLogLevel,
    EbpfCollectorConfig, FileObservationConfig, NetworkControlConfig, PayloadConfig,
    PluginAlertRuntimeConfig, ProcessSeccompConfig, ResourceMetricsConfig, SeccompNotifyConfig,
    SemanticRetentionConfig, StorageRetentionConfig, TraceFinalizationConfig,
    launch_seccomp_requirements,
};
use ebpf_collector::EbpfCollector;
use ebpf_collector::procfs::{ProcfsIdentityReader, ProcfsTreeSnapshotter};
use export_core::ExportRuntime;
use process_identity::ProcessIdentityManager;
use provider_label::ProviderClassifier;
use semantic_action_runtime::LiveSemanticActionRuntime;
use storage_core::StorageBackend;

use crate::profiles::DaemonProfileRegistry;
use crate::services::alert_ingress::AlertIngress;
use crate::services::application_protocol::ApplicationProtocolAnalyzer;
use crate::services::command_control::CommandControlService;
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::enforcement::FanotifyEnforcementService;
use crate::services::network_control::NetworkControlService;
use crate::services::payload_gate::{PayloadBodyRetentionGate, SocketHttpPayloadGate};
use crate::services::post_trace::{PostTraceBroker, PostTraceCoordinator};
use crate::services::process_seccomp::ProcessSeccompService;
use crate::services::resource_metrics::ResourceMetricsSampler;
use crate::services::retention::StorageRetentionService;
use crate::services::seccomp_notify::SeccompNotifyService;
use crate::services::seccomp_socket::SeccompSocketService;
use crate::services::seccomp_tls::SeccompTlsService;
use crate::services::tls_sync::TlsSyncService;
use crate::services::workload_diagnostics::WorkloadDiagnostics;

use super::StorageAttachService;
use super::helpers::NoopProviderClassifier;

impl StorageAttachService {
    pub(in crate::services) fn new(
        profiles: DaemonProfileRegistry,
        storage: Box<dyn StorageBackend>,
        ebpf_config: EbpfCollectorConfig,
        payload_config: PayloadConfig,
        diagnostic_log_level: DiagnosticLogLevel,
        seccomp_notify: SeccompNotifyConfig,
        process_seccomp: ProcessSeccompConfig,
        agent_invocation: AgentInvocationConfig,
        semantic_retention: SemanticRetentionConfig,
        file_observation: FileObservationConfig,
        application_protocol: ApplicationProtocolConfig,
        resource_metrics: ResourceMetricsConfig,
        storage_retention: StorageRetentionConfig,
        plugin_alert_runtime: PluginAlertRuntimeConfig,
        trace_finalization: TraceFinalizationConfig,
        workload_diagnostics: WorkloadDiagnostics,
        enforcement: FanotifyEnforcementService,
        command_control: CommandControlConfig,
        network_control: NetworkControlConfig,
        export_runtime: ExportRuntime,
    ) -> Result<Self, control_contract::reply::ControlError> {
        Self::new_with_provider_classifier(
            profiles,
            storage,
            ebpf_config,
            payload_config,
            diagnostic_log_level,
            seccomp_notify,
            process_seccomp,
            agent_invocation,
            semantic_retention,
            file_observation,
            application_protocol,
            resource_metrics,
            storage_retention,
            plugin_alert_runtime,
            trace_finalization,
            workload_diagnostics,
            enforcement,
            command_control,
            network_control,
            export_runtime,
            Box::new(NoopProviderClassifier),
            false,
        )
    }

    pub(in crate::services) fn new_with_provider_classifier(
        profiles: DaemonProfileRegistry,
        mut storage: Box<dyn StorageBackend>,
        ebpf_config: EbpfCollectorConfig,
        payload_config: PayloadConfig,
        diagnostic_log_level: DiagnosticLogLevel,
        seccomp_notify_config: SeccompNotifyConfig,
        process_seccomp_config: ProcessSeccompConfig,
        agent_invocation: AgentInvocationConfig,
        semantic_retention: SemanticRetentionConfig,
        file_observation: FileObservationConfig,
        application_protocol: ApplicationProtocolConfig,
        resource_metrics: ResourceMetricsConfig,
        storage_retention_config: StorageRetentionConfig,
        plugin_alert_runtime: PluginAlertRuntimeConfig,
        trace_finalization: TraceFinalizationConfig,
        workload_diagnostics: WorkloadDiagnostics,
        enforcement: FanotifyEnforcementService,
        command_control_config: CommandControlConfig,
        network_control_config: NetworkControlConfig,
        export_runtime: ExportRuntime,
        provider_classifier: Box<dyn ProviderClassifier>,
        provider_classification_enabled: bool,
    ) -> Result<Self, control_contract::reply::ControlError> {
        let process_records = storage.list_process_records().map_err(storage_error)?;
        let block_size = process_id_block_size()?;
        let (block_start, block_end) = storage
            .reserve_process_id_block(block_size)
            .map_err(storage_error)?;
        let process_registry =
            ProcessIdentityManager::with_reserved_block(block_start, block_end, process_records)
                .map_err(|error| {
                    control_contract::reply::ControlError::new(
                        "process_registry_init",
                        format!("{error:?}"),
                    )
                })?;
        let payload_tls_enabled = payload_config.tls.enabled;
        let payload_tls_redaction_policy = payload_config.tls.redaction_policy;
        let payload_tls_retention_max_bytes_per_trace =
            payload_config.tls.retention_max_bytes_per_trace;
        let payload_stdio_enabled = payload_config.stdio.enabled;
        let payload_stdio_redaction_policy = payload_config.stdio.redaction_policy;
        let payload_stdio_retention_max_bytes_per_trace =
            payload_config.stdio.retention_max_bytes_per_trace;
        let payload_stdio_stdin_storage_mode = payload_config.stdio.stdin_storage_mode;
        let payload_stdio_stdout_storage_mode = payload_config.stdio.stdout_storage_mode;
        let payload_stdio_stderr_storage_mode = payload_config.stdio.stderr_storage_mode;
        let payload_socket_enabled = payload_config.socket.enabled;
        let payload_socket_redaction_policy = payload_config.socket.redaction_policy;
        let payload_socket_retention_max_bytes_per_trace =
            payload_config.socket.retention_max_bytes_per_trace;
        let launch_seccomp_requirements = launch_seccomp_requirements(
            &payload_config,
            &process_seccomp_config,
            &network_control_config,
        );
        let socket_payload_gate = SocketHttpPayloadGate::new(
            payload_config.socket.http_sniff_max_bytes,
            payload_config.socket.stream_state_max_entries,
        );
        let payload_body_retention_gate = PayloadBodyRetentionGate::new(
            application_protocol.http2_max_data_preview_bytes,
            semantic_retention.clone(),
        );
        let finalization_traces_per_cycle = usize::try_from(trace_finalization.traces_per_cycle)
            .map_err(|error| {
                control_contract::reply::ControlError::new(
                    "trace_finalization_config",
                    format!("finalization_traces_per_cycle overflow: {error}"),
                )
            })?;
        let seccomp_notify = SeccompNotifyService::new(&seccomp_notify_config);
        let seccomp_tls = SeccompTlsService::new(&payload_config.tls, diagnostic_log_level);
        let tls_sync = TlsSyncService::new(&payload_config.tls)?;
        let seccomp_socket = SeccompSocketService::new(&payload_config.socket);
        let process_seccomp = ProcessSeccompService::new(&process_seccomp_config);
        let command_control = CommandControlService::new(&command_control_config)?;
        let network_control = NetworkControlService::new(&network_control_config)?;
        let post_trace_broker = PostTraceBroker::new(trace_finalization.post_trace)?;
        let post_trace_coordinator = PostTraceCoordinator::new(trace_finalization.post_trace)?;
        let alert_ingress = AlertIngress::new(plugin_alert_runtime, storage.as_mut())?;
        Ok(Self {
            profiles,
            launch_seccomp_requirements,
            storage,
            process_registry,
            collector: EbpfCollector::new(
                ebpf_config,
                payload_config,
                file_observation.bulk_read.fast_path.clone(),
            ),
            host_ebpf_preflight: Default::default(),
            identity_reader: ProcfsIdentityReader,
            snapshotter: ProcfsTreeSnapshotter,
            next_event_id: 0,
            next_diagnostic_id: 0,
            next_payload_segment_id: 0,
            payload_tls_enabled,
            diagnostic_log_level,
            last_payload_tls_diagnostics: None,
            payload_tls_redaction_policy,
            payload_tls_retention_max_bytes_per_trace,
            payload_stdio_enabled,
            payload_stdio_redaction_policy,
            payload_stdio_retention_max_bytes_per_trace,
            payload_stdio_stdin_storage_mode,
            payload_stdio_stdout_storage_mode,
            payload_stdio_stderr_storage_mode,
            payload_socket_enabled,
            payload_socket_redaction_policy,
            payload_socket_retention_max_bytes_per_trace,
            socket_payload_gate,
            payload_body_retention_gate,
            seccomp_notify,
            seccomp_tls,
            tls_sync,
            seccomp_socket,
            process_seccomp,
            command_control,
            network_control,
            pending_process_seccomp_observations: Vec::new(),
            semantic_retention: semantic_retention.clone(),
            file_observation: file_observation.clone(),
            application_protocol: ApplicationProtocolAnalyzer::new_with_retention(
                application_protocol,
                semantic_retention.clone(),
            ),
            resource_metrics: ResourceMetricsSampler::new(resource_metrics),
            storage_retention: StorageRetentionService::new(storage_retention_config),
            enforcement,
            control_plugins: ControlPluginRuntime::new(),
            plugin_configs: Default::default(),
            semantic_actions: LiveSemanticActionRuntime::new(
                agent_invocation,
                semantic_retention,
                file_observation,
            ),
            export_runtime,
            alert_ingress,
            post_trace_broker,
            post_trace_coordinator,
            workload_diagnostics,
            retained_payload_bytes_by_trace: Default::default(),
            finalized_terminal_traces: Default::default(),
            pending_terminal_finalizations: Default::default(),
            terminal_finalization_queued_at: Default::default(),
            finalization_traces_per_cycle,
            finalization_poll_interval: std::time::Duration::from_millis(
                trace_finalization.poll_interval_ms,
            ),
            terminal_settle_delay: std::time::Duration::from_millis(
                trace_finalization.settle_delay_ms,
            ),
            diagnosed_terminal_open_memberships: Default::default(),
            provider_classifier,
            provider_classification_enabled,
        })
    }
}

pub(super) const DEFAULT_PROCESS_ID_BLOCK_SIZE: u64 = 4096;
const PROCESS_ID_BLOCK_SIZE_ENV: &str = "ACTRAIL_PROCESS_ID_BLOCK_SIZE";

pub(super) fn process_id_block_size() -> Result<u64, control_contract::reply::ControlError> {
    let Some(value) = std::env::var_os(PROCESS_ID_BLOCK_SIZE_ENV) else {
        return Ok(DEFAULT_PROCESS_ID_BLOCK_SIZE);
    };
    let value = value.to_string_lossy();
    let parsed = value.parse::<u64>().map_err(|error| {
        control_contract::reply::ControlError::new(
            "process_id_block_size",
            format!("invalid {PROCESS_ID_BLOCK_SIZE_ENV}={value}: {error}"),
        )
    })?;
    if parsed == 0 {
        return Err(control_contract::reply::ControlError::new(
            "process_id_block_size",
            format!("{PROCESS_ID_BLOCK_SIZE_ENV} must be greater than zero"),
        ));
    }
    Ok(parsed)
}

fn storage_error(error: storage_core::StorageError) -> control_contract::reply::ControlError {
    control_contract::reply::ControlError::new(error.stage, error.message)
}
