use crate::daemon::{
    DaemonEvent, DaemonEventOwner, SandboxAgentDaemonBootstrap, SbDaemonConfig,
    SbDaemonConfigOverrides, SbOutput,
};

use super::{SandboxConnectClient, SbInvocation, parse_args};

pub fn run_from_env() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(error) => {
            SbOutput::startup_error(&*error);
            1
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match parse_args()? {
        SbInvocation::Daemon(invocation) => {
            let config = SbDaemonConfig::load(&invocation.config_path)?;
            let events = DaemonEventOwner::block_shutdown_signals()?;
            let mut process = SandboxAgentDaemonBootstrap::start(config)?;
            process.report_ready();
            loop {
                match events.wait(process.control_health_raw_fd(), process.diagnostics_wait())? {
                    DaemonEvent::StopRequested => break,
                    DaemonEvent::ControlServerExited => process.reap_control_server(),
                    DaemonEvent::DiagnosticsDue => process.report_diagnostics(),
                }
            }
            process.shutdown()?;
        }
        SbInvocation::Connect(invocation) => {
            let response = SandboxConnectClient::new(invocation)?.connect()?;
            SbOutput::connect_succeeded(response);
        }
        SbInvocation::Init(invocation) => {
            let mut overrides = SbDaemonConfigOverrides::new();
            if !invocation.root_process_names.is_empty() {
                overrides = overrides.with_root_process_names(invocation.root_process_names);
            }
            if let Some(path) = invocation.control_socket {
                overrides = overrides.with_control_socket_path(path);
            }
            if let Some(path) = invocation.instance_lock_path {
                overrides = overrides.with_instance_lock_path(path);
            }
            SbDaemonConfig::write_default(&invocation.output, overrides, invocation.force)?;
            SbOutput::config_written(&invocation.output);
        }
    }
    Ok(())
}
