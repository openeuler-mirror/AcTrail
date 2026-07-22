//! Host boundary for long-lived daemon services.

use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::time::{Duration, SystemTime};

use control_contract::command::{ControlCommand, TrackAddCommand};
use control_contract::reply::{
    ControlError, ControlReply, DoctorReply, LaunchTlsPlanReply, PluginCommandReply,
    PluginConfigReply, PluginConfigValidationReply, TraceListItem, TrackAddReply,
};
use control_contract::selector::TraceSelector;
use model_core::ids::{ProfileName, RequestId};
use model_core::process::ProcessIdentity;
use plugin_system::PluginInstanceStatus;
use process_identity::ProcessIdentityReader;
use uds_control_server::{ControlService, PeerCredentials};

use crate::peer_identity::{PeerIdentity, peer_error};
use crate::runtime_wiring::DaemonRuntimeWiring;

pub trait AttachService {
    fn host_pid_for_process(&self, process: ProcessIdentity) -> Result<u32, ControlError>;
    fn resolve_launch_permissions(
        &mut self,
        command: &control_contract::command::ResolveLaunchPermissionsCommand,
        host_ebpf_available: bool,
    ) -> Result<control_contract::reply::LaunchPermissionsReply, ControlError>;
    fn host_ebpf_available_for_profile(&self, profile_name: &ProfileName) -> bool;
    fn resolve_launch_tls_plan(
        &mut self,
        command: &control_contract::command::ResolveLaunchTlsPlanCommand,
    ) -> Result<LaunchTlsPlanReply, ControlError>;
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
    fn shutdown(&mut self) -> Result<(), ControlError>;
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
    fn plugin_config(&self, instance_id: &str) -> Result<PluginConfigReply, ControlError>;
    fn validate_plugin_config(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigValidationReply, ControlError>;
    fn update_plugin_config(
        &mut self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigReply, ControlError>;
}

pub trait AttachDebugService {
    fn ebpf_debug_snapshot(
        &self,
        pid: u32,
    ) -> Result<ebpf_collector::EbpfCollectorDebugSnapshot, ControlError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LaunchAdmission {
    track_add_request_id: RequestId,
    selected_profile_name: ProfileName,
}

pub struct DaemonServiceHost<A> {
    wiring: DaemonRuntimeWiring<A>,
    pending_launch_admissions: BTreeMap<model_core::process::ProcessObservation, LaunchAdmission>,
}

impl<A> DaemonServiceHost<A> {
    pub fn new(wiring: DaemonRuntimeWiring<A>) -> Self {
        Self {
            wiring,
            pending_launch_admissions: BTreeMap::new(),
        }
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

    pub fn shutdown(&mut self) -> Result<(), ControlError>
    where
        A: AttachService,
    {
        self.wiring.attach_service.shutdown()
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
    fn handle_from_peer(
        &mut self,
        credentials: PeerCredentials,
        command: ControlCommand,
    ) -> Result<ControlReply, ControlError> {
        let command_name = control_command_name(&command);
        let peer = PeerIdentity::resolve(credentials).map_err(|error| {
            audit_peer_rejection(credentials, command_name, &error);
            error
        })?;
        let removed_trace = match &command {
            ControlCommand::TrackRemove(command) => {
                match self.resolve_remove_trace_id(&peer, &command.selector) {
                    Ok(trace_id) => Some(trace_id),
                    Err(error) => {
                        audit_peer_rejection(credentials, command_name, &error);
                        return Err(error);
                    }
                }
            }
            _ => None,
        };
        if let Err(error) = self.authorize_peer_command(&peer, &command, removed_trace) {
            audit_peer_rejection(credentials, command_name, &error);
            return Err(error);
        }
        if let ControlCommand::TrackAdd(track_add) = &command
            && let Err(error) = self.consume_launch_admission(&peer, track_add)
        {
            audit_peer_rejection(credentials, command_name, &error);
            return Err(error);
        }
        let launch_admission = match &command {
            ControlCommand::ResolveLaunchPermissions(command) => {
                Some((peer.process.clone(), next_request_id(command.request_id)?))
            }
            _ => None,
        };

        let mut reply = self.handle(command)?;
        if let (
            Some((peer_process, track_add_request_id)),
            ControlReply::LaunchPermissions(permissions),
        ) = (launch_admission, &reply)
        {
            self.prune_stale_launch_admissions();
            self.pending_launch_admissions.insert(
                peer_process,
                LaunchAdmission {
                    track_add_request_id,
                    selected_profile_name: permissions.selected_profile_name.clone(),
                },
            );
        }
        match &mut reply {
            ControlReply::TrackAdded(added) => {
                self.wiring
                    .trace_runtime
                    .bind_trace_owner(added.trace_id, peer.principal.trace_owner())
                    .map_err(|error| ControlError::new("bind_trace_owner", format!("{error:?}")))?;
            }
            ControlReply::TraceList(items) if !peer.is_trusted_host_root() => {
                items.retain(|item| {
                    self.wiring
                        .trace_runtime
                        .get_trace(item.trace_id)
                        .and_then(|entry| entry.owner.as_ref())
                        .is_some_and(|owner| {
                            peer.authorize_trace_owner(item.trace_id, owner).is_ok()
                        })
                });
            }
            _ => {}
        }
        Ok(reply)
    }

    fn handle(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError> {
        match command {
            ControlCommand::ResolveLaunchPermissions(command) => {
                let host_ebpf_available = self
                    .wiring
                    .attach_service
                    .host_ebpf_available_for_profile(&command.profile_name);
                self.wiring
                    .attach_service
                    .resolve_launch_permissions(&command, host_ebpf_available)
                    .map(ControlReply::LaunchPermissions)
            }
            ControlCommand::ResolveLaunchTlsPlan(command) => self
                .wiring
                .attach_service
                .resolve_launch_tls_plan(&command)
                .map(ControlReply::LaunchTlsPlan),
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
                            .map(|selector| selector.matches(trace, None))
                            .unwrap_or(true)
                    })
                    .map(|trace| {
                        Ok(TraceListItem {
                            trace_id: trace.trace_id,
                            display_name: trace.display_name.clone(),
                            root_pid: self
                                .wiring
                                .attach_service
                                .host_pid_for_process(trace.root_process_identity)?,
                            lifecycle_state: trace.lifecycle_state,
                            health: trace.health,
                            tags: trace.tags.clone(),
                            created_at: trace.timings.created_at,
                        })
                    })
                    .collect::<Result<Vec<_>, ControlError>>()?;
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
            ControlCommand::PluginConfigGet(command) => self
                .wiring
                .attach_service
                .plugin_config(&command.instance_id)
                .map(ControlReply::PluginConfig),
            ControlCommand::PluginConfigValidate(command) => self
                .wiring
                .attach_service
                .validate_plugin_config(&command.instance_id, &command.config_json)
                .map(ControlReply::PluginConfigValidation),
            ControlCommand::PluginConfigUpdate(command) => self
                .wiring
                .attach_service
                .update_plugin_config(&command.instance_id, &command.config_json)
                .map(ControlReply::PluginConfig),
        }
    }
}

impl<A> DaemonServiceHost<A>
where
    A: AttachService,
{
    fn consume_launch_admission(
        &mut self,
        peer: &PeerIdentity,
        command: &TrackAddCommand,
    ) -> Result<(), ControlError> {
        if !command.launch_mode {
            return Ok(());
        }
        let admission = self
            .pending_launch_admissions
            .remove(&peer.process)
            .ok_or_else(|| {
                ControlError::new(
                    "launch_admission",
                    "launch-mode track-add requires a matching daemon permission decision",
                )
            })?;
        if admission.track_add_request_id != command.request_id {
            return Err(ControlError::new(
                "launch_admission",
                format!(
                    "track-add request {} does not match admitted request {}",
                    command.request_id, admission.track_add_request_id
                ),
            ));
        }
        if admission.selected_profile_name != command.profile_name {
            return Err(ControlError::new(
                "launch_admission",
                format!(
                    "track-add profile {} does not match daemon-selected profile {}",
                    command.profile_name, admission.selected_profile_name
                ),
            ));
        }
        Ok(())
    }

    fn prune_stale_launch_admissions(&mut self) {
        let identity_reader = ebpf_collector::procfs::ProcfsIdentityReader;
        self.pending_launch_admissions.retain(|process, _| {
            let Some(host) = process.host.as_ref() else {
                return false;
            };
            identity_reader
                .read_identity(host.pid)
                .is_ok_and(|current| {
                    current.host.as_ref().is_some_and(|current_host| {
                        current_host.start_time_ticks == host.start_time_ticks
                    })
                })
        });
    }

    fn authorize_peer_command(
        &self,
        peer: &PeerIdentity,
        command: &ControlCommand,
        removed_trace: Option<model_core::ids::TraceId>,
    ) -> Result<(), ControlError> {
        match command {
            ControlCommand::ResolveLaunchPermissions(_)
            | ControlCommand::ResolveLaunchTlsPlan(_) => Ok(()),
            ControlCommand::TrackAdd(command) => peer.authorize_process_ref(&command.root),
            ControlCommand::RegisterSeccompListener(command) => {
                peer.authorize_process_ref(&command.target)?;
                self.authorize_trace_owner(peer, command.trace_id)
            }
            ControlCommand::TrackRemove(_) => self.authorize_trace_owner(
                peer,
                removed_trace.ok_or_else(|| peer_error("track remove trace was not resolved"))?,
            ),
            ControlCommand::ListTraces(_) | ControlCommand::Doctor(_) => Ok(()),
            ControlCommand::PluginList(_)
            | ControlCommand::PluginStatus(_)
            | ControlCommand::PluginLoad(_)
            | ControlCommand::PluginUnload(_)
            | ControlCommand::PluginCommand(_)
            | ControlCommand::PluginConfigGet(_)
            | ControlCommand::PluginConfigValidate(_)
            | ControlCommand::PluginConfigUpdate(_) => {
                if peer.is_trusted_host_root() {
                    Ok(())
                } else {
                    Err(peer_error(
                        "plugin administration requires a host root peer",
                    ))
                }
            }
        }
    }

    fn resolve_remove_trace_id(
        &self,
        peer: &PeerIdentity,
        selector: &TraceSelector,
    ) -> Result<model_core::ids::TraceId, ControlError> {
        if peer.is_trusted_host_root() {
            return resolve_trace_id(&self.wiring.trace_runtime, selector);
        }
        self.wiring
            .trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| selector.matches(trace, None))
            .find_map(|trace| {
                let owner = self
                    .wiring
                    .trace_runtime
                    .get_trace(trace.trace_id)?
                    .owner
                    .as_ref()?;
                peer.authorize_trace_owner(trace.trace_id, owner)
                    .ok()
                    .map(|_| trace.trace_id)
            })
            .ok_or_else(|| peer_error("trace is not available to this peer"))
    }

    fn authorize_trace_owner(
        &self,
        peer: &PeerIdentity,
        trace_id: model_core::ids::TraceId,
    ) -> Result<(), ControlError> {
        if peer.is_trusted_host_root() {
            return Ok(());
        }
        let owner = self
            .wiring
            .trace_runtime
            .get_trace(trace_id)
            .and_then(|entry| entry.owner.as_ref())
            .ok_or_else(|| peer_error(format!("trace {trace_id} has no live peer binding")))?;
        peer.authorize_trace_owner(trace_id, owner)
    }
}

fn next_request_id(request_id: RequestId) -> Result<RequestId, ControlError> {
    request_id
        .get()
        .checked_add(1)
        .map(RequestId::new)
        .ok_or_else(|| ControlError::new("launch_admission", "request id overflow"))
}

fn control_command_name(command: &ControlCommand) -> &'static str {
    match command {
        ControlCommand::ResolveLaunchPermissions(_) => "resolve_launch_permissions",
        ControlCommand::ResolveLaunchTlsPlan(_) => "resolve_launch_tls_plan",
        ControlCommand::TrackAdd(_) => "track_add",
        ControlCommand::RegisterSeccompListener(_) => "register_seccomp_listener",
        ControlCommand::TrackRemove(_) => "track_remove",
        ControlCommand::ListTraces(_) => "list_traces",
        ControlCommand::Doctor(_) => "doctor",
        ControlCommand::PluginList(_) => "plugin_list",
        ControlCommand::PluginStatus(_) => "plugin_status",
        ControlCommand::PluginLoad(_) => "plugin_load",
        ControlCommand::PluginUnload(_) => "plugin_unload",
        ControlCommand::PluginCommand(_) => "plugin_command",
        ControlCommand::PluginConfigGet(_) => "plugin_config_get",
        ControlCommand::PluginConfigValidate(_) => "plugin_config_validate",
        ControlCommand::PluginConfigUpdate(_) => "plugin_config_update",
    }
}

fn audit_peer_rejection(peer: PeerCredentials, command: &'static str, error: &ControlError) {
    tracing::warn!(
        target: "actrail::peer_auth",
        peer_pid = peer.pid,
        peer_uid = peer.uid,
        peer_gid = peer.gid,
        command,
        error_code = %error.code,
        error = %error.message,
        "rejected control socket peer"
    );
}

fn resolve_trace_id(
    runtime: &trace_runtime::TraceRuntime,
    selector: &TraceSelector,
) -> Result<model_core::ids::TraceId, ControlError> {
    runtime
        .list_trace_records()
        .into_iter()
        .find(|trace| selector.matches(trace, None))
        .map(|trace| trace.trace_id)
        .ok_or_else(|| ControlError::new("not_found", "no trace matched selector"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::os::fd::RawFd;
    use std::time::{Duration, SystemTime};

    use config_core::capture_profile::CaptureProfile;
    use config_core::daemon::DEFAULT_ACTIVE_TRACE_MAX;
    use config_core::trace_snapshot::CaptureProfileSnapshot;
    use control_contract::command::{
        ControlCommand, DeploymentPermissionMode, DoctorCommand, ProcessRef,
        ResolveLaunchPermissionsCommand, TrackAddCommand,
    };
    use control_contract::reply::{
        ControlError, ControlReply, LaunchPermissionsReply, LaunchTlsPlanReply,
        LaunchTlsPlanStatus, PluginCommandReply, TrackAddReply,
    };
    use control_contract::selector::TraceSelector;
    use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};
    use model_core::process::{NamespaceIdentity, ProcessIdentity};
    use plugin_system::PluginInstanceStatus;
    use trace_runtime::commands::TrackTraceRequest;
    use uds_control_server::{ControlService, PeerCredentials};

    use crate::peer_identity::{PeerIdentity, PeerPrincipal};
    use crate::runtime_wiring::DaemonRuntimeWiring;

    use super::{AttachService, DaemonServiceHost};

    #[derive(Default)]
    struct CountingAttachService {
        drain_count: u64,
        last_host_ebpf_available: Option<bool>,
    }

    fn current_peer_credentials() -> PeerCredentials {
        PeerCredentials {
            pid: std::process::id(),
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        }
    }

    fn current_process_ref() -> ProcessRef {
        let namespace = std::fs::read_link("/proc/self/ns/pid").expect("current pid namespace");
        ProcessRef::new(
            std::process::id(),
            NamespaceIdentity::new(namespace.display().to_string()),
        )
    }

    fn launch_track_add(request_id: u64, profile_name: &str) -> ControlCommand {
        ControlCommand::TrackAdd(TrackAddCommand {
            request_id: RequestId::new(request_id),
            root: current_process_ref(),
            display_name: TraceName::new("launch-admission-test"),
            profile_name: ProfileName::new(profile_name),
            tags: BTreeSet::new(),
            launch_mode: true,
            initial_suppressed_fds: Vec::new(),
        })
    }

    fn test_host() -> DaemonServiceHost<CountingAttachService> {
        DaemonServiceHost::new(DaemonRuntimeWiring {
            trace_runtime: trace_runtime::TraceRuntime::new(Vec::new(), 1),
            attach_service: CountingAttachService::default(),
            active_trace_max: DEFAULT_ACTIVE_TRACE_MAX,
            available_collectors: vec!["ebpf".to_string()],
            loaded_policy_plugins: Vec::new(),
            storage_ready: true,
        })
    }

    impl AttachService for CountingAttachService {
        fn host_pid_for_process(&self, process: ProcessIdentity) -> Result<u32, ControlError> {
            u32::try_from(process.get())
                .map_err(|error| ControlError::new("test_process", error.to_string()))
        }

        fn resolve_launch_permissions(
            &mut self,
            command: &control_contract::command::ResolveLaunchPermissionsCommand,
            host_ebpf_available: bool,
        ) -> Result<LaunchPermissionsReply, ControlError> {
            self.last_host_ebpf_available = Some(host_ebpf_available);
            Ok(LaunchPermissionsReply {
                requested_host_ebpf: command.host_ebpf,
                requested_seccomp_notify: command.seccomp_notify,
                selected_host_ebpf: host_ebpf_available,
                selected_seccomp_notify: command.seccomp_notify_available,
                selected_profile_name: command.profile_name.clone(),
                payload_tls_seccomp: false,
                payload_socket_seccomp: false,
                process_seccomp: false,
                network_control_seccomp: false,
                file_mkdir_seccomp: false,
                file_rmdir_seccomp: false,
                required_capabilities: Vec::new(),
                degraded: false,
                reasons: Vec::new(),
            })
        }

        fn host_ebpf_available_for_profile(&self, _profile_name: &ProfileName) -> bool {
            true
        }

        fn resolve_launch_tls_plan(
            &mut self,
            _command: &control_contract::command::ResolveLaunchTlsPlanCommand,
        ) -> Result<LaunchTlsPlanReply, ControlError> {
            Ok(LaunchTlsPlanReply {
                status: LaunchTlsPlanStatus::Unsupported {
                    reason: "unused".to_string(),
                },
                cache_hit: false,
                resolve_elapsed_micros: 0,
            })
        }

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

        fn shutdown(&mut self) -> Result<(), ControlError> {
            Ok(())
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

        fn plugin_config(&self, _instance_id: &str) -> Result<PluginConfigReply, ControlError> {
            Err(ControlError::new("unused", "unused"))
        }

        fn validate_plugin_config(
            &self,
            _instance_id: &str,
            _config_json: &str,
        ) -> Result<PluginConfigValidationReply, ControlError> {
            Err(ControlError::new("unused", "unused"))
        }

        fn update_plugin_config(
            &mut self,
            _instance_id: &str,
            _config_json: &str,
        ) -> Result<PluginConfigReply, ControlError> {
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

    #[test]
    fn launch_permissions_use_daemon_collector_availability() {
        let wiring = DaemonRuntimeWiring {
            trace_runtime: trace_runtime::TraceRuntime::new(Vec::new(), 1),
            attach_service: CountingAttachService::default(),
            active_trace_max: DEFAULT_ACTIVE_TRACE_MAX,
            available_collectors: vec!["ebpf".to_string()],
            loaded_policy_plugins: Vec::new(),
            storage_ready: true,
        };
        let mut host = DaemonServiceHost::new(wiring);

        let reply = host
            .handle(ControlCommand::ResolveLaunchPermissions(
                ResolveLaunchPermissionsCommand {
                    request_id: RequestId::new(1),
                    profile_name: ProfileName::new("default"),
                    host_ebpf: DeploymentPermissionMode::Auto,
                    seccomp_notify: DeploymentPermissionMode::Auto,
                    seccomp_notify_available: false,
                    seccomp_notify_detail: "denied".to_string(),
                },
            ))
            .expect("resolve launch permissions");

        let ControlReply::LaunchPermissions(reply) = reply else {
            panic!("unexpected reply");
        };
        assert!(reply.selected_host_ebpf);
        assert!(!reply.selected_seccomp_notify);
        assert_eq!(
            host.wiring.attach_service.last_host_ebpf_available,
            Some(true)
        );
    }

    #[test]
    fn launch_mode_track_add_requires_a_daemon_admission() {
        let mut host = test_host();

        let error = host
            .handle_from_peer(current_peer_credentials(), launch_track_add(2, "default"))
            .unwrap_err();

        assert_eq!(error.code, "launch_admission");
        assert!(error.message.contains("permission decision"));
    }

    #[test]
    fn launch_admission_rejects_a_client_selected_profile() {
        let mut host = test_host();
        let peer = current_peer_credentials();
        host.handle_from_peer(
            peer,
            ControlCommand::ResolveLaunchPermissions(ResolveLaunchPermissionsCommand {
                request_id: RequestId::new(10),
                profile_name: ProfileName::new("default"),
                host_ebpf: DeploymentPermissionMode::Auto,
                seccomp_notify: DeploymentPermissionMode::Auto,
                seccomp_notify_available: false,
                seccomp_notify_detail: "denied".to_string(),
            }),
        )
        .expect("permission decision");

        let error = host
            .handle_from_peer(peer, launch_track_add(11, "other"))
            .unwrap_err();

        assert_eq!(error.code, "launch_admission");
        assert!(error.message.contains("does not match daemon-selected"));
    }

    #[test]
    fn launch_admission_is_bound_to_the_next_request_and_consumed_once() {
        let mut host = test_host();
        let peer = current_peer_credentials();
        host.handle_from_peer(
            peer,
            ControlCommand::ResolveLaunchPermissions(ResolveLaunchPermissionsCommand {
                request_id: RequestId::new(20),
                profile_name: ProfileName::new("default"),
                host_ebpf: DeploymentPermissionMode::Auto,
                seccomp_notify: DeploymentPermissionMode::Auto,
                seccomp_notify_available: false,
                seccomp_notify_detail: "denied".to_string(),
            }),
        )
        .expect("permission decision");

        let request_error = host
            .handle_from_peer(peer, launch_track_add(22, "default"))
            .unwrap_err();
        assert_eq!(request_error.code, "launch_admission");
        assert!(
            request_error
                .message
                .contains("does not match admitted request")
        );

        host.handle_from_peer(
            peer,
            ControlCommand::ResolveLaunchPermissions(ResolveLaunchPermissionsCommand {
                request_id: RequestId::new(30),
                profile_name: ProfileName::new("default"),
                host_ebpf: DeploymentPermissionMode::Auto,
                seccomp_notify: DeploymentPermissionMode::Auto,
                seccomp_notify_available: false,
                seccomp_notify_detail: "denied".to_string(),
            }),
        )
        .expect("replacement permission decision");

        let attach_error = host
            .handle_from_peer(peer, launch_track_add(31, "default"))
            .unwrap_err();
        assert_eq!(attach_error.code, "unused");

        let replay_error = host
            .handle_from_peer(peer, launch_track_add(31, "default"))
            .unwrap_err();
        assert_eq!(replay_error.code, "launch_admission");
    }

    #[test]
    fn track_remove_hides_trace_existence_from_a_different_container() {
        let wiring = DaemonRuntimeWiring {
            trace_runtime: trace_runtime::TraceRuntime::new(Vec::new(), 1),
            attach_service: CountingAttachService::default(),
            active_trace_max: DEFAULT_ACTIVE_TRACE_MAX,
            available_collectors: Vec::new(),
            loaded_policy_plugins: Vec::new(),
            storage_ready: true,
        };
        let mut host = DaemonServiceHost::new(wiring);
        let trace_id = host.wiring.trace_runtime.reserve_trace_id();
        let profile = CaptureProfile::new(ProfileName::new("test"), Vec::new());
        let profile_snapshot =
            CaptureProfileSnapshot::from_profile(&profile, SystemTime::UNIX_EPOCH);
        let sensor_plan = host
            .wiring
            .trace_runtime
            .negotiate(&profile_snapshot)
            .unwrap();
        host.wiring
            .trace_runtime
            .create_starting_trace(
                trace_id,
                TrackTraceRequest {
                    root_identity: ProcessIdentity::new(1),
                    root_container_id: Some("container-a".to_string()),
                    root_working_directory: None,
                    display_name: TraceName::new("owned"),
                    profile_snapshot,
                    tags: BTreeSet::new(),
                    created_at: SystemTime::UNIX_EPOCH,
                },
                sensor_plan,
            )
            .unwrap();
        let owner = PeerPrincipal {
            uid: 0,
            container_id: Some("container-a".to_string()),
            pid_namespace: "pid:[101]".to_string(),
            host_pid_namespace: false,
        };
        host.wiring
            .trace_runtime
            .bind_trace_owner(trace_id, owner.trace_owner())
            .unwrap();
        let owner_peer = PeerIdentity {
            credentials: PeerCredentials {
                pid: 201,
                uid: 0,
                gid: 0,
            },
            process: model_core::process::ProcessObservation::default(),
            principal: owner,
        };
        assert_eq!(
            host.resolve_remove_trace_id(&owner_peer, &TraceSelector::TraceId(trace_id)),
            Ok(trace_id)
        );
        let peer = PeerIdentity {
            credentials: PeerCredentials {
                pid: 202,
                uid: 0,
                gid: 0,
            },
            process: model_core::process::ProcessObservation::default(),
            principal: PeerPrincipal {
                uid: 0,
                container_id: Some("container-b".to_string()),
                pid_namespace: "pid:[202]".to_string(),
                host_pid_namespace: false,
            },
        };
        let existing_error = host
            .resolve_remove_trace_id(&peer, &TraceSelector::TraceId(trace_id))
            .unwrap_err();
        let missing_error = host
            .resolve_remove_trace_id(&peer, &TraceSelector::TraceId(TraceId::new(999)))
            .unwrap_err();

        assert_eq!(existing_error, missing_error);
        assert_eq!(existing_error.code, "peer_identity");
        assert_eq!(
            existing_error.message,
            "trace is not available to this peer"
        );

        let trusted_root = PeerIdentity {
            credentials: PeerCredentials {
                pid: 1,
                uid: 0,
                gid: 0,
            },
            process: model_core::process::ProcessObservation::default(),
            principal: PeerPrincipal {
                uid: 0,
                container_id: None,
                pid_namespace: "pid:[1]".to_string(),
                host_pid_namespace: true,
            },
        };
        let root_error = host
            .resolve_remove_trace_id(&trusted_root, &TraceSelector::TraceId(TraceId::new(999)))
            .unwrap_err();
        assert_eq!(root_error.code, "not_found");
    }
}
