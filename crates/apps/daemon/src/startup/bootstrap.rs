//! Startup sequencing for the daemon application.

use std::io;
use std::net::SocketAddr;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::time::Duration;

use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, CommandControlConfig, DiagnosticLogLevel,
    EbpfCollectorConfig, EnforcementConfig, FileObservationConfig, HandObservationConfig,
    NetworkControlConfig, PayloadConfig, PluginAlertRuntimeConfig, ProcessSeccompConfig,
    ResourceMetricsConfig, SandboxEvidenceConfig, SandboxEvidenceSynchronousConfig,
    SeccompNotifyConfig, SemanticRetentionConfig, StorageRetentionConfig, TraceFinalizationConfig,
    WorkloadDiagnosticsConfig,
};
use config_core::provider_rules::ProviderRuleSetConfig;
use control_contract::command::PluginLoadCommand;
use control_contract::reply::ControlError;
use plugin_system::PluginInstanceStatus;
use sandbox_evidence_sqlite::{
    SandboxEvidenceSqliteConfig, SandboxEvidenceSqliteStore, SandboxEvidenceSynchronous,
};
use sandbox_upstream_transport::{UpstreamServerConfig, UpstreamTcpServer};
use storage_factory::StorageConfig;
use uds_control_server::{UdsControlConnection, UdsControlServer};

use crate::profiles::DaemonProfileRegistry;
use crate::runtime_wiring::DaemonRuntimeWiring;
use crate::service_host::{AttachService, DaemonServiceHost};
use crate::services::attach::StorageAttachService;
use crate::services::workload_diagnostics::WorkloadDiagnostics;
use crate::services::{
    build_runtime_wiring_with_provider_rule_set_and_storage_retention,
    build_runtime_wiring_with_storage_retention,
};

pub struct DaemonBootstrap<A> {
    wiring: DaemonRuntimeWiring<A>,
}

impl<A> DaemonBootstrap<A>
where
    A: AttachService,
{
    pub fn new(wiring: DaemonRuntimeWiring<A>) -> Self {
        Self { wiring }
    }

    pub fn build_control_server(self) -> UdsControlServer<DaemonServiceHost<A>> {
        UdsControlServer::new(DaemonServiceHost::new(self.wiring))
    }
}

pub struct LocalDaemonServer {
    server: UdsControlServer<DaemonServiceHost<StorageAttachService>>,
    workload_diagnostics: WorkloadDiagnostics,
    hand_observation_server: Option<UpstreamTcpServer>,
    sandbox_evidence_store: Option<SandboxEvidenceSqliteStore>,
}

impl LocalDaemonServer {
    pub fn build(
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
        shutdown_runtime_timeout_ms: u64,
        workload_diagnostics_config: WorkloadDiagnosticsConfig,
        enforcement: EnforcementConfig,
        command_control: CommandControlConfig,
        network_control: NetworkControlConfig,
    ) -> Result<Self, ControlError> {
        let workload_diagnostics = WorkloadDiagnostics::new(workload_diagnostics_config);
        let wiring = build_runtime_wiring_with_storage_retention(
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
            shutdown_runtime_timeout_ms,
            workload_diagnostics.clone(),
            enforcement,
            command_control,
            network_control,
        )?;
        workload_diagnostics.start();
        Ok(Self {
            server: DaemonBootstrap::new(wiring).build_control_server(),
            workload_diagnostics,
            hand_observation_server: None,
            sandbox_evidence_store: None,
        })
    }

    pub fn build_with_provider_rule_set(
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
        shutdown_runtime_timeout_ms: u64,
        workload_diagnostics_config: WorkloadDiagnosticsConfig,
        enforcement: EnforcementConfig,
        command_control: CommandControlConfig,
        network_control: NetworkControlConfig,
        provider_rule_set: &ProviderRuleSetConfig,
    ) -> Result<Self, ControlError> {
        let workload_diagnostics = WorkloadDiagnostics::new(workload_diagnostics_config);
        let wiring = build_runtime_wiring_with_provider_rule_set_and_storage_retention(
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
            shutdown_runtime_timeout_ms,
            workload_diagnostics.clone(),
            enforcement,
            command_control,
            network_control,
            provider_rule_set,
        )?;
        workload_diagnostics.start();
        Ok(Self {
            server: DaemonBootstrap::new(wiring).build_control_server(),
            workload_diagnostics,
            hand_observation_server: None,
            sandbox_evidence_store: None,
        })
    }

    pub fn start_hand_observation(
        &mut self,
        config: &HandObservationConfig,
        evidence: &SandboxEvidenceConfig,
    ) -> Result<Option<SocketAddr>, ControlError> {
        if !config.enabled {
            return Ok(None);
        }
        if self.hand_observation_server.is_some() {
            return Err(ControlError::new(
                "hand_observation",
                "Hand observation listener is already running",
            ));
        }
        if self.sandbox_evidence_store.is_some() {
            return Err(ControlError::new(
                "sandbox_evidence",
                "sandbox evidence store is already running",
            ));
        }
        let store_config = SandboxEvidenceSqliteConfig {
            path: evidence.path.clone(),
            schema_version: evidence.schema_version,
            create_parent_directory: evidence.create_parent_directory,
            busy_timeout: Duration::from_millis(evidence.busy_timeout_ms),
            writer_queue_capacity: evidence.writer_queue_capacity,
            batch_max_observations: evidence.batch_max_observations,
            transaction_max_batches: evidence.transaction_max_batches,
            flush_interval: Duration::from_millis(evidence.flush_interval_ms),
            retention_max_observations: evidence.retention_max_observations,
            capacity_max_bytes: evidence.capacity_max_bytes,
            synchronous: match evidence.synchronous {
                SandboxEvidenceSynchronousConfig::Normal => SandboxEvidenceSynchronous::Normal,
                SandboxEvidenceSynchronousConfig::Full => SandboxEvidenceSynchronous::Full,
            },
            wal_autocheckpoint_pages: evidence.wal_autocheckpoint_pages,
            shutdown_drain_timeout: Duration::from_millis(evidence.shutdown_drain_timeout_ms),
            writer_thread_stack_bytes: evidence.writer_thread_stack_bytes,
            read_limit_max: evidence.read_limit_max,
        };
        let mut store = SandboxEvidenceSqliteStore::start(store_config)
            .map_err(|error| ControlError::new("sandbox_evidence", error))?;
        let server_config = UpstreamServerConfig {
            listen_addr: config.listen_addr,
            max_connections: config.max_gateway_connections,
            accept_poll_interval: Duration::from_millis(config.accept_poll_interval_ms),
            connection_poll_interval: Duration::from_millis(config.connection_poll_interval_ms),
            connection_idle_timeout: Duration::from_millis(config.connection_idle_timeout_ms),
            write_timeout: Duration::from_millis(config.write_timeout_ms),
            read_buffer_bytes: config.read_buffer_bytes,
            connection_thread_stack_bytes: config.connection_thread_stack_bytes,
        };
        let sink = Arc::new(
            self.server
                .service_mut()
                .sandbox_route_sink(store.write_port()),
        );
        let server = match UpstreamTcpServer::start(server_config, sink) {
            Ok(server) => server,
            Err(error) => {
                let _ = store.shutdown();
                return Err(ControlError::new(
                    "hand_observation",
                    format!("{}: {}", error.stage(), error.message()),
                ));
            }
        };
        let local_addr = server.local_addr();
        self.sandbox_evidence_store = Some(store);
        self.hand_observation_server = Some(server);
        Ok(Some(local_addr))
    }

    pub fn handle_request(&mut self, request: &[u8]) -> Vec<u8> {
        self.server.handle_bytes(request)
    }

    pub fn load_plugin(
        &mut self,
        command: PluginLoadCommand,
    ) -> Result<PluginInstanceStatus, ControlError> {
        self.server.service_mut().load_plugin(command)
    }

    pub fn drain_live_events(&mut self) -> Result<(), ControlError> {
        self.server.service_mut().drain_live_events()
    }

    pub fn shutdown(&mut self) -> Result<(), ControlError> {
        let hand_result = match self.hand_observation_server.as_mut() {
            Some(server) => server
                .shutdown()
                .map_err(|error| ControlError::new("hand_observation_shutdown", error.to_string())),
            None => Ok(()),
        };
        self.hand_observation_server = None;
        let evidence_result = match self.sandbox_evidence_store.as_mut() {
            Some(store) => store
                .shutdown()
                .map_err(|error| ControlError::new("sandbox_evidence_shutdown", error.to_string())),
            None => Ok(()),
        };
        self.sandbox_evidence_store = None;
        let daemon_result = self.server.service_mut().shutdown();
        hand_result.and(evidence_result).and(daemon_result)
    }

    pub fn ebpf_debug_snapshot(
        &mut self,
        pid: u32,
    ) -> Result<ebpf_collector::EbpfCollectorDebugSnapshot, ControlError> {
        self.server.service_mut().ebpf_debug_snapshot(pid)
    }

    pub(crate) fn progress_control_connection(
        &mut self,
        connection: &mut UdsControlConnection,
    ) -> io::Result<bool> {
        connection.try_progress(&mut self.server)
    }

    pub(crate) fn control_event_poll_fds(&mut self) -> Result<Vec<RawFd>, ControlError> {
        self.server.service_mut().event_poll_fds()
    }

    pub(crate) fn background_poll_timeout(&mut self) -> Result<Option<Duration>, ControlError> {
        self.server.service_mut().background_poll_timeout()
    }

    pub(crate) fn workload_diagnostics(&self) -> &WorkloadDiagnostics {
        &self.workload_diagnostics
    }
}
