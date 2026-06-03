//! Construction for the SQLite-backed attach service.

use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, DiagnosticLogLevel, EbpfCollectorConfig,
    PayloadConfig, ProcessSeccompConfig, ResourceMetricsConfig, SeccompNotifyConfig,
};
use ebpf_collector::EbpfCollector;
use ebpf_collector::procfs::{ProcfsIdentityReader, ProcfsTreeSnapshotter};
use provider_label::ProviderClassifier;
use semantic_action_runtime::LiveSemanticActionRuntime;
use sqlite_storage::SqliteStorage;

use crate::profiles::DaemonProfileRegistry;
use crate::services::application_protocol::ApplicationProtocolAnalyzer;
use crate::services::enforcement::FanotifyEnforcementService;
use crate::services::live::otel_export::LiveOtelExporter;
use crate::services::payload_gate::SocketHttpPayloadGate;
use crate::services::process_seccomp::ProcessSeccompService;
use crate::services::resource_metrics::ResourceMetricsSampler;
use crate::services::seccomp_notify::SeccompNotifyService;
use crate::services::seccomp_socket::SeccompSocketService;
use crate::services::seccomp_tls::SeccompTlsService;
use crate::services::tls_sync::TlsSyncService;

use super::SqliteAttachService;
use super::helpers::NoopProviderClassifier;

impl SqliteAttachService {
    pub(in crate::services) fn new(
        profiles: DaemonProfileRegistry,
        storage: SqliteStorage,
        ebpf_config: EbpfCollectorConfig,
        payload_config: PayloadConfig,
        diagnostic_log_level: DiagnosticLogLevel,
        seccomp_notify: SeccompNotifyConfig,
        process_seccomp: ProcessSeccompConfig,
        agent_invocation: AgentInvocationConfig,
        application_protocol: ApplicationProtocolConfig,
        resource_metrics: ResourceMetricsConfig,
        enforcement: FanotifyEnforcementService,
        live_otel_export: LiveOtelExporter,
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
            application_protocol,
            resource_metrics,
            enforcement,
            live_otel_export,
            Box::new(NoopProviderClassifier),
            false,
        )
    }

    pub(in crate::services) fn new_with_provider_classifier(
        profiles: DaemonProfileRegistry,
        storage: SqliteStorage,
        ebpf_config: EbpfCollectorConfig,
        payload_config: PayloadConfig,
        diagnostic_log_level: DiagnosticLogLevel,
        seccomp_notify_config: SeccompNotifyConfig,
        process_seccomp_config: ProcessSeccompConfig,
        agent_invocation: AgentInvocationConfig,
        application_protocol: ApplicationProtocolConfig,
        resource_metrics: ResourceMetricsConfig,
        enforcement: FanotifyEnforcementService,
        live_otel_export: LiveOtelExporter,
        provider_classifier: Box<dyn ProviderClassifier>,
        provider_classification_enabled: bool,
    ) -> Result<Self, control_contract::reply::ControlError> {
        let payload_tls_enabled = payload_config.tls.enabled;
        let payload_tls_redaction_policy = payload_config.tls.redaction_policy;
        let payload_tls_retention_max_bytes_per_trace =
            payload_config.tls.retention_max_bytes_per_trace;
        let payload_stdio_enabled = payload_config.stdio.enabled;
        let payload_stdio_redaction_policy = payload_config.stdio.redaction_policy;
        let payload_stdio_retention_max_bytes_per_trace =
            payload_config.stdio.retention_max_bytes_per_trace;
        let payload_socket_enabled = payload_config.socket.enabled;
        let payload_socket_redaction_policy = payload_config.socket.redaction_policy;
        let payload_socket_retention_max_bytes_per_trace =
            payload_config.socket.retention_max_bytes_per_trace;
        let socket_payload_gate = SocketHttpPayloadGate::new(
            payload_config.socket.http_sniff_max_bytes,
            payload_config.socket.stream_state_max_entries,
        );
        let seccomp_notify = SeccompNotifyService::new(&seccomp_notify_config);
        let seccomp_tls = SeccompTlsService::new(&payload_config.tls, diagnostic_log_level);
        let tls_sync = TlsSyncService::new(&payload_config.tls)?;
        let seccomp_socket = SeccompSocketService::new(&payload_config.socket);
        let process_seccomp = ProcessSeccompService::new(&process_seccomp_config);
        Ok(Self {
            profiles,
            storage,
            collector: EbpfCollector::new(ebpf_config, payload_config),
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
            payload_socket_enabled,
            payload_socket_redaction_policy,
            payload_socket_retention_max_bytes_per_trace,
            socket_payload_gate,
            seccomp_notify,
            seccomp_tls,
            tls_sync,
            seccomp_socket,
            process_seccomp,
            pending_process_seccomp_observations: Vec::new(),
            application_protocol: ApplicationProtocolAnalyzer::new(application_protocol),
            resource_metrics: ResourceMetricsSampler::new(resource_metrics),
            enforcement,
            semantic_actions: LiveSemanticActionRuntime::new(agent_invocation),
            live_otel_export,
            provider_classifier,
            provider_classification_enabled,
        })
    }
}
