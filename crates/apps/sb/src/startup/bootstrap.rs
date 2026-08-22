use std::io;
use std::sync::Arc;

use sandbox_agent_runtime::{GuestResourceSource, ProcessIoSource, SandboxAgent, SandboxTransport};
use sandbox_linux_collector::{LinuxResourceReader, SandboxProcessIoCollector};
use sandbox_vsock_transport::VsockClient;

use super::config::ValidatedSbConfig;
use super::instance_lock::InstanceLock;
use crate::SbConfig;

pub struct SandboxAgentBootstrap;

pub struct SandboxAgentProcess {
    agent: SandboxAgent,
    _instance_lock: InstanceLock,
}

impl SandboxAgentBootstrap {
    pub fn start(config: SbConfig) -> io::Result<SandboxAgentProcess> {
        let config = config.validate()?;
        Self::start_validated(config)
    }

    fn start_validated(config: ValidatedSbConfig) -> io::Result<SandboxAgentProcess> {
        let instance_lock = InstanceLock::acquire(&config.instance_lock_path)?;
        let io_collector = SandboxProcessIoCollector::start(config.linux)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let resource_reader = LinuxResourceReader::open(config.resource_procfs_root)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let vsock_client = VsockClient::new(config.transport)?;
        let transport: Arc<dyn SandboxTransport> = Arc::new(VsockTransport { vsock_client });
        let agent = SandboxAgent::start(
            config.runtime,
            Box::new(IoSource { io_collector }),
            Box::new(ResourceSource { resource_reader }),
            transport,
        )?;
        Ok(SandboxAgentProcess {
            agent,
            _instance_lock: instance_lock,
        })
    }
}

impl SandboxAgentProcess {
    pub fn agent(&self) -> &SandboxAgent {
        &self.agent
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        self.agent.shutdown()
    }
}

struct IoSource {
    io_collector: SandboxProcessIoCollector,
}

impl ProcessIoSource for IoSource {
    fn poll(&mut self) -> io::Result<Vec<sandbox_observation::ProcessIoCounters>> {
        let cycle = self.io_collector.poll();
        for failure in cycle.failures {
            eprintln!("actrail-sb collector: {failure}");
        }
        if cycle.kernel_diagnostics.pending_io_drops > 0
            || cycle.kernel_diagnostics.aggregate_drops > 0
            || cycle.kernel_diagnostics.descendant_tracking_drops > 0
        {
            eprintln!(
                "actrail-sb collector drops pending={} aggregate={} descendants={}",
                cycle.kernel_diagnostics.pending_io_drops,
                cycle.kernel_diagnostics.aggregate_drops,
                cycle.kernel_diagnostics.descendant_tracking_drops
            );
        }
        Ok(cycle.process_io)
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

struct VsockTransport {
    vsock_client: VsockClient,
}

impl SandboxTransport for VsockTransport {
    fn connect(&self) -> io::Result<Box<dyn sandbox_agent_runtime::SandboxConnection>> {
        self.vsock_client
            .connect()
            .map(|connection| Box::new(connection) as Box<_>)
    }
}
