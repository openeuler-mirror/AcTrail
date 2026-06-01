//! Host boundary for long-lived daemon services.

use std::os::fd::RawFd;
use std::time::{Duration, SystemTime};

use control_contract::command::ControlCommand;
use control_contract::reply::{
    ControlError, ControlReply, DoctorReply, TraceListItem, TrackAddReply,
};
use control_contract::selector::TraceSelector;
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
}

impl<A> ControlService for DaemonServiceHost<A>
where
    A: AttachService,
{
    fn handle(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError> {
        self.drain_live_events()?;
        match command {
            ControlCommand::TrackAdd(command) => self
                .wiring
                .attach_service
                .attach_existing(&mut self.wiring.trace_runtime, &command)
                .map(ControlReply::TrackAdded),
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
