//! Construction of daemon runtime wiring from concrete adapters.

use collector_capability::CollectorDescriptor;
use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, CommandControlConfig, DiagnosticLogLevel,
    EbpfCollectorConfig, EnforcementConfig, FileObservationConfig, NetworkControlConfig,
    PayloadConfig, PluginAlertRuntimeConfig, ProcessSeccompConfig, ResourceMetricsConfig,
    RuntimeExportConfig, SeccompNotifyConfig, SemanticRetentionConfig, StorageRetentionConfig,
    TraceFinalizationConfig,
};
use config_core::provider_rules::ProviderRuleSetConfig;
use control_contract::reply::ControlError;
use export_core::ExportRuntime;
use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};
use model_core::ids::CollectorName;
use provider_label::ProviderClassifier;
use rule_set_provider::classifier::RuleSetClassifier;
use rule_set_provider::config::RuleSetAdapterConfig;
use rule_set_provider::loader::load_rules;
use storage_core::StorageOpenMode;
use storage_factory::{StorageConfig, open_storage_backend};

use crate::profiles::DaemonProfileRegistry;
use crate::runtime_wiring::DaemonRuntimeWiring;

use super::application_protocol::COLLECTOR_NAME as APPLICATION_PROTOCOL_COLLECTOR_NAME;
use super::attach::StorageAttachService;
use super::enforcement::{
    COLLECTOR_NAME as ENFORCEMENT_COLLECTOR_NAME, FanotifyEnforcementService,
    enforcement_descriptor,
};
use super::process_seccomp::PROCESS_SECCOMP_COLLECTOR_NAME;
use super::resource_metrics::COLLECTOR_NAME as RESOURCE_METRICS_COLLECTOR_NAME;
use super::workload_diagnostics::WorkloadDiagnostics;

const TLS_SYNC_COLLECTOR_NAME: &str = "tls-sync";

#[cfg(test)]
pub(crate) fn build_runtime_wiring(
    storage_config: &StorageConfig,
    profiles: DaemonProfileRegistry,
    ebpf_config: EbpfCollectorConfig,
    payload_config: PayloadConfig,
    active_trace_max: u32,
    diagnostic_log_level: DiagnosticLogLevel,
    seccomp_notify: SeccompNotifyConfig,
    process_seccomp: ProcessSeccompConfig,
    agent_invocation: AgentInvocationConfig,
    semantic_retention: SemanticRetentionConfig,
    file_observation: FileObservationConfig,
    application_protocol: ApplicationProtocolConfig,
    resource_metrics: ResourceMetricsConfig,
    trace_finalization: TraceFinalizationConfig,
    workload_diagnostics: WorkloadDiagnostics,
    export_runtime: RuntimeExportConfig,
    enforcement: EnforcementConfig,
    command_control: CommandControlConfig,
    network_control: NetworkControlConfig,
) -> Result<DaemonRuntimeWiring<StorageAttachService>, ControlError> {
    build_runtime_wiring_with_storage_retention(
        storage_config,
        profiles,
        ebpf_config,
        payload_config,
        active_trace_max,
        diagnostic_log_level,
        seccomp_notify,
        process_seccomp,
        agent_invocation,
        semantic_retention,
        file_observation,
        application_protocol,
        resource_metrics,
        StorageRetentionConfig::default(),
        PluginAlertRuntimeConfig::default(),
        trace_finalization,
        workload_diagnostics,
        export_runtime,
        enforcement,
        command_control,
        network_control,
    )
}

pub(crate) fn build_runtime_wiring_with_storage_retention(
    storage_config: &StorageConfig,
    profiles: DaemonProfileRegistry,
    ebpf_config: EbpfCollectorConfig,
    payload_config: PayloadConfig,
    active_trace_max: u32,
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
    export_runtime: RuntimeExportConfig,
    enforcement: EnforcementConfig,
    command_control: CommandControlConfig,
    network_control: NetworkControlConfig,
) -> Result<DaemonRuntimeWiring<StorageAttachService>, ControlError> {
    build_runtime_wiring_with_attach_service(
        storage_config,
        profiles,
        ebpf_config,
        payload_config,
        active_trace_max,
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
        export_runtime,
        enforcement,
        command_control,
        network_control,
        None,
    )
}

pub(crate) fn build_runtime_wiring_with_provider_rule_set_and_storage_retention(
    storage_config: &StorageConfig,
    profiles: DaemonProfileRegistry,
    ebpf_config: EbpfCollectorConfig,
    payload_config: PayloadConfig,
    active_trace_max: u32,
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
    export_runtime: RuntimeExportConfig,
    enforcement: EnforcementConfig,
    command_control: CommandControlConfig,
    network_control: NetworkControlConfig,
    provider_rule_set: &ProviderRuleSetConfig,
) -> Result<DaemonRuntimeWiring<StorageAttachService>, ControlError> {
    let rules = load_rules(&provider_rule_set.rules_path)
        .map_err(|message| ControlError::new("provider_rules", message))?;
    let classifier = RuleSetClassifier::new(RuleSetAdapterConfig::from(provider_rule_set), rules);
    build_runtime_wiring_with_attach_service(
        storage_config,
        profiles,
        ebpf_config,
        payload_config,
        active_trace_max,
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
        export_runtime,
        enforcement,
        command_control,
        network_control,
        Some(Box::new(classifier)),
    )
}

fn build_runtime_wiring_with_attach_service(
    storage_config: &StorageConfig,
    profiles: DaemonProfileRegistry,
    ebpf_config: EbpfCollectorConfig,
    payload_config: PayloadConfig,
    active_trace_max: u32,
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
    export_runtime_config: RuntimeExportConfig,
    enforcement_config: EnforcementConfig,
    command_control_config: CommandControlConfig,
    network_control_config: NetworkControlConfig,
    provider_classifier: Option<Box<dyn ProviderClassifier>>,
) -> Result<DaemonRuntimeWiring<StorageAttachService>, ControlError> {
    let storage = open_storage_backend(storage_config, StorageOpenMode::ReadWrite)
        .map_err(|error| ControlError::new(error.stage, error.message))?;
    let trace_id_seed = storage
        .next_trace_id_seed()
        .map_err(|error| ControlError::new(error.stage, error.message))?;
    let event_id_seed = storage
        .next_event_id_seed()
        .map_err(|error| ControlError::new(error.stage, error.message))?;
    let diagnostic_id_seed = storage
        .next_diagnostic_id_seed()
        .map_err(|error| ControlError::new(error.stage, error.message))?;
    let payload_segment_id_seed = storage
        .next_payload_segment_id_seed()
        .map_err(|error| ControlError::new(error.stage, error.message))?;
    let enforcement = FanotifyEnforcementService::new(enforcement_config.clone())?;
    let export_runtime = export_runtime(export_runtime_config)?;

    let mut attach_service = match provider_classifier {
        Some(provider_classifier) => StorageAttachService::new_with_provider_classifier(
            profiles.clone(),
            storage,
            ebpf_config.clone(),
            payload_config.clone(),
            diagnostic_log_level,
            seccomp_notify.clone(),
            process_seccomp.clone(),
            agent_invocation.clone(),
            semantic_retention.clone(),
            file_observation.clone(),
            application_protocol.clone(),
            resource_metrics.clone(),
            storage_retention.clone(),
            plugin_alert_runtime,
            trace_finalization,
            workload_diagnostics.clone(),
            enforcement,
            command_control_config.clone(),
            network_control_config.clone(),
            export_runtime,
            provider_classifier,
            true,
        )?,
        None => StorageAttachService::new(
            profiles.clone(),
            storage,
            ebpf_config.clone(),
            payload_config.clone(),
            diagnostic_log_level,
            seccomp_notify.clone(),
            process_seccomp.clone(),
            agent_invocation.clone(),
            semantic_retention.clone(),
            file_observation.clone(),
            application_protocol.clone(),
            resource_metrics.clone(),
            storage_retention,
            plugin_alert_runtime,
            trace_finalization,
            workload_diagnostics.clone(),
            enforcement,
            command_control_config.clone(),
            network_control_config.clone(),
            export_runtime,
        )?,
    };
    attach_service.set_id_seeds(event_id_seed, diagnostic_id_seed);
    attach_service.set_payload_segment_id_seed(payload_segment_id_seed);
    attach_service.preflight_host_ebpf();
    let mut available_collectors = Vec::new();
    if ebpf_config.enabled
        && attach_service.collector_ready()
        && attach_service.any_host_ebpf_preflight_available()
    {
        available_collectors.push(attach_service.collector_name());
    }
    if payload_config.tls.enabled && payload_config.tls.capture_backend.is_sync() {
        available_collectors.push(TLS_SYNC_COLLECTOR_NAME.to_string());
    }
    if application_protocol.enabled {
        available_collectors.push(APPLICATION_PROTOCOL_COLLECTOR_NAME.to_string());
    }
    if resource_metrics.enabled {
        available_collectors.push(RESOURCE_METRICS_COLLECTOR_NAME.to_string());
    }
    if enforcement_config.enabled && attach_service.enforcement.enabled() {
        available_collectors.push(ENFORCEMENT_COLLECTOR_NAME.to_string());
    }
    if process_seccomp.enabled {
        available_collectors.push(PROCESS_SECCOMP_COLLECTOR_NAME.to_string());
    }

    let mut collector_descriptors = Vec::new();
    if process_seccomp.enabled {
        collector_descriptors.push(process_seccomp_descriptor());
    }
    if ebpf_config.enabled {
        collector_descriptors.push(attach_service.collector_descriptor());
    }
    if payload_config.tls.enabled && payload_config.tls.capture_backend.is_sync() {
        collector_descriptors.push(tls_sync_descriptor());
    }
    if application_protocol.enabled {
        collector_descriptors.push(application_protocol_descriptor(&application_protocol));
    }
    if resource_metrics.enabled {
        collector_descriptors.push(resource_metrics_descriptor());
    }
    if enforcement_config.enabled {
        collector_descriptors.push(enforcement_descriptor());
    }

    Ok(DaemonRuntimeWiring {
        trace_runtime: trace_runtime::TraceRuntime::new(collector_descriptors, trace_id_seed),
        attach_service,
        active_trace_max,
        available_collectors,
        loaded_policy_plugins: Vec::new(),
        storage_ready: true,
    })
}

fn export_runtime(config: RuntimeExportConfig) -> Result<ExportRuntime, ControlError> {
    export_factory::build_export_runtime(&config)
        .map_err(|error| ControlError::new(error.code, error.message))
}

fn process_seccomp_descriptor() -> CollectorDescriptor {
    CollectorDescriptor {
        name: CollectorName::new(PROCESS_SECCOMP_COLLECTOR_NAME),
        capabilities: vec![CapabilityDescriptor::new(
            Capability::ProcExecContext,
            vec![CapabilityField::new(
                "exec_argv_context",
                GuaranteeClass::GuaranteedByTransportCollector,
            )],
        )],
        supports_attach_coverage_guard: false,
        supports_existing_pid_attach: false,
    }
}

fn tls_sync_descriptor() -> CollectorDescriptor {
    CollectorDescriptor {
        name: CollectorName::new(TLS_SYNC_COLLECTOR_NAME),
        capabilities: vec![CapabilityDescriptor::new(
            Capability::TlsPlaintextPayload,
            vec![CapabilityField::new(
                "tls_plaintext_segment",
                GuaranteeClass::RequiresPayloadCollector,
            )],
        )],
        supports_attach_coverage_guard: false,
        supports_existing_pid_attach: false,
    }
}

fn resource_metrics_descriptor() -> CollectorDescriptor {
    CollectorDescriptor {
        name: CollectorName::new(RESOURCE_METRICS_COLLECTOR_NAME),
        capabilities: vec![CapabilityDescriptor::new(
            Capability::ResourceMetrics,
            vec![
                CapabilityField::new(
                    "process_cpu_percent",
                    GuaranteeClass::AvailableWhenMetadataObservable,
                ),
                CapabilityField::new(
                    "process_memory_rss_vsz",
                    GuaranteeClass::AvailableWhenMetadataObservable,
                ),
            ],
        )],
        supports_attach_coverage_guard: false,
        supports_existing_pid_attach: true,
    }
}

fn application_protocol_descriptor(config: &ApplicationProtocolConfig) -> CollectorDescriptor {
    let mut capabilities = Vec::new();
    if config.http1_enabled {
        capabilities.push(CapabilityDescriptor::new(
            Capability::NetApplicationPlaintextHttp,
            vec![CapabilityField::new(
                "http_1_semantic_event",
                GuaranteeClass::RequiresPayloadCollector,
            )],
        ));
    }
    if config.http2_enabled {
        capabilities.push(CapabilityDescriptor::new(
            Capability::NetApplicationHttp2Frames,
            vec![
                CapabilityField::new("http_2_frame", GuaranteeClass::RequiresPayloadCollector),
                CapabilityField::new("http_2_data", GuaranteeClass::RequiresPayloadCollector),
            ],
        ));
    }
    CollectorDescriptor {
        name: CollectorName::new(APPLICATION_PROTOCOL_COLLECTOR_NAME),
        capabilities,
        supports_attach_coverage_guard: false,
        supports_existing_pid_attach: true,
    }
}
