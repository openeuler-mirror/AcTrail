//! Startup sequencing for the daemon application.

use std::io;
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, DiagnosticLogLevel, EbpfCollectorConfig,
    EnforcementConfig, LiveOtelExportConfig, ProcessSeccompConfig, ResourceMetricsConfig,
    SeccompNotifyConfig,
};
use config_core::provider_rules::ProviderRuleSetConfig;
use control_contract::reply::ControlError;
use uds_control_server::UdsControlServer;

use crate::profiles::DaemonProfileRegistry;
use crate::runtime_wiring::DaemonRuntimeWiring;
use crate::service_host::{AttachService, DaemonServiceHost};
use crate::services::attach::SqliteAttachService;
use crate::services::{build_runtime_wiring, build_runtime_wiring_with_provider_rule_set};

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
    server: UdsControlServer<DaemonServiceHost<SqliteAttachService>>,
}

impl LocalDaemonServer {
    pub fn build(
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
    ) -> Result<Self, ControlError> {
        let wiring = build_runtime_wiring(
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
        )?;
        Ok(Self {
            server: DaemonBootstrap::new(wiring).build_control_server(),
        })
    }

    pub fn build_with_provider_rule_set(
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
    ) -> Result<Self, ControlError> {
        let wiring = build_runtime_wiring_with_provider_rule_set(
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
            provider_rule_set,
        )?;
        Ok(Self {
            server: DaemonBootstrap::new(wiring).build_control_server(),
        })
    }

    pub fn handle_request(&mut self, request: &[u8]) -> Vec<u8> {
        self.server.handle_bytes(request)
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

    pub(crate) fn serve_connection(&mut self, stream: &mut UnixStream) -> io::Result<()> {
        self.server.serve_connection(stream)
    }

    pub(crate) fn control_event_poll_fds(&mut self) -> Result<Vec<RawFd>, ControlError> {
        self.server.service_mut().event_poll_fds()
    }

    pub(crate) fn background_poll_timeout(&mut self) -> Result<Option<Duration>, ControlError> {
        self.server.service_mut().background_poll_timeout()
    }
}
