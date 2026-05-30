//! Construction of daemon runtime wiring from concrete adapters.

use std::path::Path;

use collector_capability::CollectorDescriptor;
use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, DiagnosticLogLevel, EbpfCollectorConfig,
    EnforcementConfig, LiveOtelExportConfig, ProcessSeccompConfig, ResourceMetricsConfig,
    SeccompNotifyConfig,
};
use config_core::provider_rules::ProviderRuleSetConfig;
use control_contract::reply::ControlError;
use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};
use model_core::ids::CollectorName;
use provider_label::ProviderClassifier;
use rule_set_provider::classifier::RuleSetClassifier;
use rule_set_provider::config::RuleSetAdapterConfig;
use rule_set_provider::loader::load_rules;
use sqlite_storage::SqliteStorage;

use crate::profiles::DaemonProfileRegistry;
use crate::runtime_wiring::DaemonRuntimeWiring;

use super::application_protocol::COLLECTOR_NAME as APPLICATION_PROTOCOL_COLLECTOR_NAME;
use super::attach::SqliteAttachService;
use super::enforcement::{
    COLLECTOR_NAME as ENFORCEMENT_COLLECTOR_NAME, FanotifyEnforcementService,
    enforcement_descriptor,
};
use super::live::otel_export::LiveOtelExporter;
use super::process_seccomp::PROCESS_SECCOMP_COLLECTOR_NAME;
use super::resource_metrics::COLLECTOR_NAME as RESOURCE_METRICS_COLLECTOR_NAME;

pub(crate) fn build_runtime_wiring(
    storage_path: &Path,
    profiles: DaemonProfileRegistry,
    ebpf_config: EbpfCollectorConfig,
    diagnostic_log_level: DiagnosticLogLevel,
    seccomp_notify: SeccompNotifyConfig,
    process_seccomp: ProcessSeccompConfig,
    agent_invocation: AgentInvocationConfig,
    application_protocol: ApplicationProtocolConfig,
    resource_metrics: ResourceMetricsConfig,
    live_otel_export: LiveOtelExportConfig,
    enforcement: EnforcementConfig,
) -> Result<DaemonRuntimeWiring<SqliteAttachService>, ControlError> {
    build_runtime_wiring_with_attach_service(
        storage_path,
        profiles,
        ebpf_config,
        diagnostic_log_level,
        seccomp_notify,
        process_seccomp,
        agent_invocation,
        application_protocol,
        resource_metrics,
        live_otel_export,
        enforcement,
        None,
    )
}

pub(crate) fn build_runtime_wiring_with_provider_rule_set(
    storage_path: &Path,
    profiles: DaemonProfileRegistry,
    ebpf_config: EbpfCollectorConfig,
    diagnostic_log_level: DiagnosticLogLevel,
    seccomp_notify: SeccompNotifyConfig,
    process_seccomp: ProcessSeccompConfig,
    agent_invocation: AgentInvocationConfig,
    application_protocol: ApplicationProtocolConfig,
    resource_metrics: ResourceMetricsConfig,
    live_otel_export: LiveOtelExportConfig,
    enforcement: EnforcementConfig,
    provider_rule_set: &ProviderRuleSetConfig,
) -> Result<DaemonRuntimeWiring<SqliteAttachService>, ControlError> {
    let rules = load_rules(&provider_rule_set.rules_path)
        .map_err(|message| ControlError::new("provider_rules", message))?;
    let classifier = RuleSetClassifier::new(RuleSetAdapterConfig::from(provider_rule_set), rules);
    build_runtime_wiring_with_attach_service(
        storage_path,
        profiles,
        ebpf_config,
        diagnostic_log_level,
        seccomp_notify,
        process_seccomp,
        agent_invocation,
        application_protocol,
        resource_metrics,
        live_otel_export,
        enforcement,
        Some(Box::new(classifier)),
    )
}

fn build_runtime_wiring_with_attach_service(
    storage_path: &Path,
    profiles: DaemonProfileRegistry,
    ebpf_config: EbpfCollectorConfig,
    diagnostic_log_level: DiagnosticLogLevel,
    seccomp_notify: SeccompNotifyConfig,
    process_seccomp: ProcessSeccompConfig,
    agent_invocation: AgentInvocationConfig,
    application_protocol: ApplicationProtocolConfig,
    resource_metrics: ResourceMetricsConfig,
    live_otel_export_config: LiveOtelExportConfig,
    enforcement_config: EnforcementConfig,
    provider_classifier: Option<Box<dyn ProviderClassifier>>,
) -> Result<DaemonRuntimeWiring<SqliteAttachService>, ControlError> {
    let storage = SqliteStorage::open(storage_path)
        .map_err(|error| ControlError::new("open_storage", error.to_string()))?;
    let trace_id_seed = storage
        .next_trace_id_seed()
        .map_err(|error| ControlError::new("trace_id_seed", error.to_string()))?;
    let event_id_seed = storage
        .next_event_id_seed()
        .map_err(|error| ControlError::new("event_id_seed", error.to_string()))?;
    let diagnostic_id_seed = storage
        .next_diagnostic_id_seed()
        .map_err(|error| ControlError::new("diagnostic_id_seed", error.to_string()))?;
    let payload_segment_id_seed = storage
        .next_payload_segment_id_seed()
        .map_err(|error| ControlError::new("payload_segment_id_seed", error.to_string()))?;
    let enforcement = FanotifyEnforcementService::new(enforcement_config.clone())?;
    let live_otel_export = LiveOtelExporter::new(live_otel_export_config)?;

    let mut attach_service = match provider_classifier {
        Some(provider_classifier) => SqliteAttachService::new_with_provider_classifier(
            profiles.clone(),
            storage.clone(),
            ebpf_config.clone(),
            diagnostic_log_level,
            seccomp_notify.clone(),
            process_seccomp.clone(),
            agent_invocation.clone(),
            application_protocol.clone(),
            resource_metrics.clone(),
            enforcement,
            live_otel_export,
            provider_classifier,
            true,
        ),
        None => SqliteAttachService::new(
            profiles.clone(),
            storage.clone(),
            ebpf_config.clone(),
            diagnostic_log_level,
            seccomp_notify.clone(),
            process_seccomp.clone(),
            agent_invocation.clone(),
            application_protocol.clone(),
            resource_metrics.clone(),
            enforcement,
            live_otel_export,
        ),
    };
    attach_service.set_id_seeds(event_id_seed, diagnostic_id_seed);
    attach_service.set_payload_segment_id_seed(payload_segment_id_seed);
    let mut available_collectors = Vec::new();
    if ebpf_config.enabled && attach_service.collector_ready() {
        available_collectors.push(attach_service.collector_name());
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
        available_collectors,
        loaded_policy_plugins: Vec::new(),
        storage_ready: true,
    })
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
