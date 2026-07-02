//! Host boundary for long-lived daemon services.

use std::os::fd::RawFd;
use std::time::{Duration, SystemTime};

use control_contract::command::ControlCommand;
use control_contract::reply::{
    ControlError, ControlReply, DoctorReply, PluginCommandReply, TraceListItem, TrackAddReply,
};
use control_contract::selector::TraceSelector;
use plugin_system::PluginInstanceStatus;
use uds_control_server::ControlService;

use crate::runtime_wiring::DaemonRuntimeWiring;

pub trait AttachService {
    fn attach_existing(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &control_contract::command::TrackAddCommand,
    ) -> Result<TrackAddReply, ControlError>;
    fn drain_live_events(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError>;
    fn event_poll_fds(&self) -> Result<Vec<RawFd>, ControlError>;
    fn background_poll_timeout(&self) -> Result<Option<Duration>, ControlError>;
    fn remove_root(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        trace_id: model_core::ids::TraceId,
        removed_at: SystemTime,
    ) -> Result<(), ControlError>;
    fn register_seccomp_listener(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: control_contract::command::RegisterSeccompListenerCommand,
    ) -> Result<(), ControlError>;
    fn plugin_statuses(&self) -> Vec<PluginInstanceStatus>;
    fn load_plugin(
        &mut self,
        command: control_contract::command::PluginLoadCommand,
    ) -> Result<PluginInstanceStatus, ControlError>;
    fn unload_plugin(&mut self, instance_id: &str) -> Result<PluginInstanceStatus, ControlError>;
    fn handle_plugin_command(
        &mut self,
        command: control_contract::command::PluginCommandCommand,
    ) -> Result<PluginCommandReply, ControlError>;
}

pub trait AttachDebugService {
    fn ebpf_debug_snapshot(
        &self,
        pid: u32,
    ) -> Result<ebpf_collector::EbpfCollectorDebugSnapshot, ControlError>;
}

pub struct DaemonServiceHost<A> {
    wiring: DaemonRuntimeWiring<A>,
}

impl<A> DaemonServiceHost<A> {
    pub fn new(wiring: DaemonRuntimeWiring<A>) -> Self {
        Self { wiring }
    }

    pub fn drain_live_events(&mut self) -> Result<(), ControlError>
    where
        A: AttachService,
    {
        self.wiring
            .attach_service
            .drain_live_events(&mut self.wiring.trace_runtime)
    }

    pub fn event_poll_fds(&self) -> Result<Vec<RawFd>, ControlError>
    where
        A: AttachService,
    {
        self.wiring.attach_service.event_poll_fds()
    }

    pub fn background_poll_timeout(&self) -> Result<Option<Duration>, ControlError>
    where
        A: AttachService,
    {
        self.wiring.attach_service.background_poll_timeout()
    }

    pub fn ebpf_debug_snapshot(
        &self,
        pid: u32,
    ) -> Result<ebpf_collector::EbpfCollectorDebugSnapshot, ControlError>
    where
        A: AttachDebugService,
    {
        self.wiring.attach_service.ebpf_debug_snapshot(pid)
    }

    pub fn load_plugin(
        &mut self,
        command: control_contract::command::PluginLoadCommand,
    ) -> Result<PluginInstanceStatus, ControlError>
    where
        A: AttachService,
    {
        self.wiring.attach_service.load_plugin(command)
    }
}

impl<A> ControlService for DaemonServiceHost<A>
where
    A: AttachService,
{
    fn handle(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError> {
        match command {
            ControlCommand::TrackAdd(command) => {
                let active_trace_count = self
                    .wiring
                    .trace_runtime
                    .list_trace_records()
                    .into_iter()
                    .filter(|trace| !trace.lifecycle_state.is_terminal())
                    .count();
                let active_trace_max =
                    usize::try_from(self.wiring.active_trace_max).map_err(|error| {
                        ControlError::new(
                            "active_trace_limit",
                            format!("active_trace_max overflow: {error}"),
                        )
                    })?;
                if active_trace_count >= active_trace_max {
                    return Err(ControlError::new(
                        "active_trace_limit",
                        format!(
                            "active trace limit reached: {active_trace_count}/{active_trace_max}"
                        ),
                    ));
                }
                self.wiring
                    .attach_service
                    .attach_existing(&mut self.wiring.trace_runtime, &command)
                    .map(ControlReply::TrackAdded)
            }
            ControlCommand::RegisterSeccompListener(command) => {
                self.wiring
                    .attach_service
                    .register_seccomp_listener(&mut self.wiring.trace_runtime, command)?;
                Ok(ControlReply::SeccompListenerRegistered)
            }
            ControlCommand::TrackRemove(command) => {
                let trace_id = resolve_trace_id(&self.wiring.trace_runtime, &command.selector)?;
                self.wiring.attach_service.remove_root(
                    &mut self.wiring.trace_runtime,
                    trace_id,
                    SystemTime::now(),
                )?;
                Ok(ControlReply::TrackRemoved)
            }
            ControlCommand::ListTraces(command) => {
                let items = self
                    .wiring
                    .trace_runtime
                    .list_trace_records()
                    .into_iter()
                    .filter(|trace| {
                        command
                            .selector
                            .as_ref()
                            .map(|selector| selector.matches(trace))
                            .unwrap_or(true)
                    })
                    .map(|trace| TraceListItem {
                        trace_id: trace.trace_id,
                        display_name: trace.display_name.clone(),
                        root_pid: trace.root_process_identity.pid,
                        lifecycle_state: trace.lifecycle_state,
                        health: trace.health,
                        tags: trace.tags.clone(),
                        created_at: trace.timings.created_at,
                    })
                    .collect();
                Ok(ControlReply::TraceList(items))
            }
            ControlCommand::Doctor(_) => Ok(ControlReply::Doctor(DoctorReply {
                available_collectors: self.wiring.available_collectors.clone(),
                loaded_policy_plugins: self.wiring.loaded_policy_plugins.clone(),
                storage_ready: self.wiring.storage_ready,
            })),
            ControlCommand::PluginList(_) => Ok(ControlReply::PluginList(
                self.wiring.attach_service.plugin_statuses(),
            )),
            ControlCommand::PluginStatus(command) => {
                let status = self
                    .wiring
                    .attach_service
                    .plugin_statuses()
                    .into_iter()
                    .find(|status| status.instance_id == command.instance_id)
                    .ok_or_else(|| {
                        ControlError::new(
                            "plugin_not_found",
                            format!("plugin instance {} not found", command.instance_id),
                        )
                    })?;
                Ok(ControlReply::PluginStatus(status))
            }
            ControlCommand::PluginLoad(command) => self
                .wiring
                .attach_service
                .load_plugin(command)
                .map(ControlReply::PluginStatus),
            ControlCommand::PluginUnload(command) => self
                .wiring
                .attach_service
                .unload_plugin(&command.instance_id)
                .map(ControlReply::PluginStatus),
            ControlCommand::PluginCommand(command) => self
                .wiring
                .attach_service
                .handle_plugin_command(command)
                .map(ControlReply::PluginCommand),
        }
    }
}

fn resolve_trace_id(
    runtime: &trace_runtime::TraceRuntime,
    selector: &TraceSelector,
) -> Result<model_core::ids::TraceId, ControlError> {
    runtime
        .list_trace_records()
        .into_iter()
        .find(|trace| selector.matches(trace))
        .map(|trace| trace.trace_id)
        .ok_or_else(|| ControlError::new("not_found", "no trace matched selector"))
}

#[cfg(test)]
mod tests {
    use std::os::fd::RawFd;
    use std::time::{Duration, SystemTime};

    use config_core::daemon::DEFAULT_ACTIVE_TRACE_MAX;
    use control_contract::command::{ControlCommand, DoctorCommand, TrackAddCommand};
    use control_contract::reply::{ControlError, PluginCommandReply, TrackAddReply};
    use model_core::ids::{RequestId, TraceId};
    use plugin_system::PluginInstanceStatus;
    use uds_control_server::ControlService;

    use crate::runtime_wiring::DaemonRuntimeWiring;

    use super::{AttachService, DaemonServiceHost};

    #[derive(Default)]
    struct CountingAttachService {
        drain_count: u64,
    }

    impl AttachService for CountingAttachService {
        fn attach_existing(
            &mut self,
            _trace_runtime: &mut trace_runtime::TraceRuntime,
            _command: &TrackAddCommand,
        ) -> Result<TrackAddReply, ControlError> {
            Err(ControlError::new("unused", "unused"))
        }

        fn drain_live_events(
            &mut self,
            _trace_runtime: &mut trace_runtime::TraceRuntime,
        ) -> Result<(), ControlError> {
            self.drain_count += 1;
            Ok(())
        }

        fn event_poll_fds(&self) -> Result<Vec<RawFd>, ControlError> {
            Ok(Vec::new())
        }

        fn background_poll_timeout(&self) -> Result<Option<Duration>, ControlError> {
            Ok(None)
        }

        fn remove_root(
            &mut self,
            _trace_runtime: &mut trace_runtime::TraceRuntime,
            _trace_id: TraceId,
            _removed_at: SystemTime,
        ) -> Result<(), ControlError> {
            Ok(())
        }

        fn register_seccomp_listener(
            &mut self,
            _trace_runtime: &mut trace_runtime::TraceRuntime,
            _command: control_contract::command::RegisterSeccompListenerCommand,
        ) -> Result<(), ControlError> {
            Ok(())
        }

        fn plugin_statuses(&self) -> Vec<PluginInstanceStatus> {
            Vec::new()
        }

        fn load_plugin(
            &mut self,
            _command: control_contract::command::PluginLoadCommand,
        ) -> Result<PluginInstanceStatus, ControlError> {
            Err(ControlError::new("unused", "unused"))
        }

        fn unload_plugin(
            &mut self,
            _instance_id: &str,
        ) -> Result<PluginInstanceStatus, ControlError> {
            Err(ControlError::new("unused", "unused"))
        }

        fn handle_plugin_command(
            &mut self,
            _command: control_contract::command::PluginCommandCommand,
        ) -> Result<PluginCommandReply, ControlError> {
            Err(ControlError::new("unused", "unused"))
        }
    }

    #[test]
    fn control_command_handling_does_not_pre_drain_live_events() {
        let wiring = DaemonRuntimeWiring {
            trace_runtime: trace_runtime::TraceRuntime::new(Vec::new(), 1),
            attach_service: CountingAttachService::default(),
            active_trace_max: DEFAULT_ACTIVE_TRACE_MAX,
            available_collectors: Vec::new(),
            loaded_policy_plugins: Vec::new(),
            storage_ready: true,
        };
        let mut host = DaemonServiceHost::new(wiring);

        let _reply = host
            .handle(ControlCommand::Doctor(DoctorCommand {
                request_id: RequestId::new(1),
            }))
            .expect("doctor reply");

        assert_eq!(host.wiring.attach_service.drain_count, 0);
    }
}
