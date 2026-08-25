use std::io;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::time::Duration;

use sandbox_agent_runtime::{
    GuestResourceSource, ProcessIoSource, SandboxAgentDaemon, SandboxTransportFactory,
};
use sandbox_control_uds::{
    SandboxControlCodec, SandboxControlConnectionLimits, SandboxControlServerHandle,
    SandboxControlUdsServer, SandboxControlUdsServerConfig,
};
use sandbox_linux_collector::{LinuxResourceReader, SandboxProcessIoCollector};
use sandbox_vsock_transport::{VsockTransportConfig, VsockTransportFactory};

use super::SbDaemonConfig;
use super::config::{ValidatedControlConfig, ValidatedSbDaemonConfig};
use super::instance_lock::InstanceLock;
use super::output::{CollectorDiagnostics, SbOutput};

pub struct SandboxAgentDaemonBootstrap;

pub struct SandboxAgentDaemonProcess {
    agent: SandboxAgentDaemon,
    control_server: Option<SandboxControlServerHandle>,
    output: SbOutput,
    _instance_lock: InstanceLock,
}

impl SandboxAgentDaemonBootstrap {
    pub fn start(config: SbDaemonConfig) -> io::Result<SandboxAgentDaemonProcess> {
        let config = config.validate()?;
        Self::start_validated(config)
    }

    fn start_validated(config: ValidatedSbDaemonConfig) -> io::Result<SandboxAgentDaemonProcess> {
        let (output, collector_diagnostics) =
            SbOutput::runtime(config.diagnostics_interval, &config.control.socket_path)?;
        let transport: Arc<dyn SandboxTransportFactory> =
            Arc::new(VsockTransportFactory::new(VsockTransportConfig {
                io_timeout: config.sender_io_timeout,
            })?);
        let control_server = Self::control_server(&config.control)?;
        let instance_lock = InstanceLock::acquire(&config.instance_lock_path)?;
        let io_collector = SandboxProcessIoCollector::start(config.linux)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let resource_reader = LinuxResourceReader::open(config.resource_procfs_root)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let agent = SandboxAgentDaemon::start(
            config.runtime,
            Box::new(IoSource {
                io_collector,
                diagnostics: collector_diagnostics,
            }),
            Box::new(ResourceSource { resource_reader }),
            transport,
        )?;
        let control_server = control_server
            .start(agent.control_port())
            .map_err(io::Error::other)?;
        Ok(SandboxAgentDaemonProcess {
            agent,
            control_server: Some(control_server),
            output,
            _instance_lock: instance_lock,
        })
    }

    fn control_server(config: &ValidatedControlConfig) -> io::Result<SandboxControlUdsServer> {
        let server = SandboxControlUdsServerConfig::new(
            config.socket_path.clone(),
            config.socket_mode,
            config.accepted_connection_max,
            config.worker_thread_stack_bytes,
        )
        .map_err(io::Error::other)?;
        let limits = SandboxControlConnectionLimits::new(
            config.max_frame_bytes,
            config.max_frame_bytes,
            config.request_timeout,
        )
        .map_err(io::Error::other)?;
        let codec = SandboxControlCodec::new(config.max_frame_bytes).map_err(io::Error::other)?;
        SandboxControlUdsServer::new(server, limits, codec).map_err(io::Error::other)
    }
}

impl SandboxAgentDaemonProcess {
    pub fn report_ready(&self) {
        self.output.report_ready(self.agent.status());
    }

    pub fn report_diagnostics(&mut self) {
        self.output.report_if_due(&self.agent);
    }

    pub fn diagnostics_wait(&self) -> Option<Duration> {
        self.output.diagnostics_wait()
    }

    pub fn control_health_raw_fd(&self) -> Option<RawFd> {
        self.control_server
            .as_ref()
            .map(SandboxControlServerHandle::health_raw_fd)
    }

    pub fn reap_control_server(&mut self) {
        let Some(server) = &mut self.control_server else {
            return;
        };
        let Some(result) = server.try_result() else {
            return;
        };
        self.output
            .report_control_server_exit(result.as_ref().err());
        self.control_server = None;
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        let stop = self
            .control_server
            .as_mut()
            .map(SandboxControlServerHandle::request_stop)
            .unwrap_or(Ok(()))
            .map_err(io::Error::other);
        let agent = self.agent.shutdown();
        let control = self
            .control_server
            .as_mut()
            .map(SandboxControlServerHandle::join)
            .unwrap_or(Ok(()))
            .map_err(io::Error::other);
        stop.and(agent).and(control)
    }
}

struct IoSource {
    io_collector: SandboxProcessIoCollector,
    diagnostics: Option<Arc<CollectorDiagnostics>>,
}

impl ProcessIoSource for IoSource {
    fn establish_baseline(&mut self) -> io::Result<()> {
        self.io_collector
            .reset_publication()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let cycle = self.io_collector.poll();
        self.record_diagnostics(&cycle);
        match cycle.failures.first() {
            Some(error) => Err(io::Error::other(format!(
                "cannot establish sandbox I/O baseline: {error}"
            ))),
            None => Ok(()),
        }
    }

    fn activate_publication(&mut self, generation: u64) -> io::Result<()> {
        self.io_collector
            .activate_publication(generation)
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn poll(&mut self) -> io::Result<Vec<sandbox_observation::Observation>> {
        let cycle = self.io_collector.poll();
        self.record_diagnostics(&cycle);
        let mut observations = Vec::with_capacity(cycle.process_io.len() + cycle.oom_victims.len());
        observations.extend(
            cycle
                .process_io
                .into_iter()
                .map(sandbox_observation::Observation::ProcessIo),
        );
        observations.extend(
            cycle
                .oom_victims
                .into_iter()
                .map(sandbox_observation::Observation::OomVictim),
        );
        Ok(observations)
    }
}

impl IoSource {
    fn record_diagnostics(&self, cycle: &sandbox_linux_collector::ProcessIoCycle) {
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.record(cycle.failures.len(), cycle.kernel_diagnostics);
        }
    }
}

struct ResourceSource {
    resource_reader: LinuxResourceReader,
}

impl GuestResourceSource for ResourceSource {
    fn sample(&mut self) -> io::Result<sandbox_observation::GuestResourceSnapshot> {
        self.resource_reader
            .sample()
            .map_err(|error| io::Error::other(error.to_string()))
    }
}
