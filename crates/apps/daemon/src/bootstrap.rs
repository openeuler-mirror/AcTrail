//! Startup sequencing for the daemon application.

use std::io;
use std::os::fd::RawFd;
use std::time::Duration;

use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, CommandControlConfig, DiagnosticLogLevel,
    EbpfCollectorConfig, EnforcementConfig, FileObservationConfig, NetworkControlConfig,
    PayloadConfig, ProcessSeccompConfig, ResourceMetricsConfig, RuntimeExportConfig,
    SeccompNotifyConfig, SemanticRetentionConfig, StorageRetentionConfig, TraceFinalizationConfig,
    WorkloadDiagnosticsConfig,
};
use config_core::provider_rules::ProviderRuleSetConfig;
use control_contract::command::PluginLoadCommand;
use control_contract::reply::ControlError;
use plugin_system::PluginInstanceStatus;
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
        trace_finalization: TraceFinalizationConfig,
        workload_diagnostics_config: WorkloadDiagnosticsConfig,
        export_runtime: RuntimeExportConfig,
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
            trace_finalization,
            workload_diagnostics.clone(),
            export_runtime,
            enforcement,
            command_control,
            network_control,
        )?;
        workload_diagnostics.start();
        Ok(Self {
            server: DaemonBootstrap::new(wiring).build_control_server(),
            workload_diagnostics,
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
        trace_finalization: TraceFinalizationConfig,
        workload_diagnostics_config: WorkloadDiagnosticsConfig,
        export_runtime: RuntimeExportConfig,
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
            trace_finalization,
            workload_diagnostics.clone(),
            export_runtime,
            enforcement,
            command_control,
            network_control,
            provider_rule_set,
        )?;
        workload_diagnostics.start();
        Ok(Self {
            server: DaemonBootstrap::new(wiring).build_control_server(),
            workload_diagnostics,
        })
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
