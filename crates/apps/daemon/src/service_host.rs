//! Host boundary for long-lived daemon services.

use std::os::fd::RawFd;
use std::time::{Duration, SystemTime};

use control_contract::command::ControlCommand;
use control_contract::reply::{
    ControlError, ControlReply, DoctorReply, PluginCommandReply, TraceListItem, TrackAddReply,
};
use control_contract::selector::TraceSelector;
use plugin_system::PluginInstanceStatus;
use uds_control_server::{ControlService, PeerCredentials};

use crate::peer_identity::{PeerIdentity, peer_error};
use crate::runtime_wiring::DaemonRuntimeWiring;

pub trait AttachService {
    fn resolve_launch_permissions(
        &mut self,
        command: &control_contract::command::ResolveLaunchPermissionsCommand,
        host_ebpf_available: bool,
    ) -> Result<control_contract::reply::LaunchPermissionsReply, ControlError>;
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
                Some(resolve_trace_id(&self.wiring.trace_runtime, &command.selector)?)
            }
            _ => None,
        };
        if let Err(error) = self.authorize_peer_command(&peer, &command, removed_trace) {
            audit_peer_rejection(credentials, command_name, &error);
            return Err(error);
        }

        let mut reply = self.handle(command)?;
        match &mut reply {
            ControlReply::TrackAdded(added) => {
                self.wiring
                    .trace_runtime
                    .bind_trace_owner(added.trace_id, peer.principal.trace_owner())
                    .map_err(|error| {
                        ControlError::new("bind_trace_owner", format!("{error:?}"))
                    })?;
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
                    .available_collectors
                    .iter()
                    .any(|collector| collector == "ebpf");
                self.wiring
                    .attach_service
                    .resolve_launch_permissions(&command, host_ebpf_available)
                    .map(ControlReply::LaunchPermissions)
            }
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

impl<A> DaemonServiceHost<A>
where
    A: AttachService,
{
    fn authorize_peer_command(
        &self,
        peer: &PeerIdentity,
        command: &ControlCommand,
        removed_trace: Option<model_core::ids::TraceId>,
    ) -> Result<(), ControlError> {
        match command {
            ControlCommand::ResolveLaunchPermissions(_) => Ok(()),
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
            | ControlCommand::PluginCommand(_) => {
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

fn control_command_name(command: &ControlCommand) -> &'static str {
    match command {
        ControlCommand::ResolveLaunchPermissions(_) => "resolve_launch_permissions",
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
    }
}

fn audit_peer_rejection(
    peer: PeerCredentials,
    command: &'static str,
    error: &ControlError,
) {
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
        .find(|trace| selector.matches(trace))
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
        ControlCommand, DeploymentPermissionMode, DoctorCommand,
        ResolveLaunchPermissionsCommand, TrackAddCommand, TrackRemoveCommand,
    };
    use control_contract::reply::{
        ControlError, ControlReply, LaunchPermissionsReply, PluginCommandReply, TrackAddReply,
    };
    use control_contract::selector::TraceSelector;
    use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};
    use model_core::process::ProcessIdentity;
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

    impl AttachService for CountingAttachService {
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
                required_capabilities: Vec::new(),
                degraded: false,
                reasons: Vec::new(),
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
    fn track_remove_rejects_a_different_container_principal() {
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
                    root_identity: ProcessIdentity::new(101, 1, 1),
                    root_container_id: Some("container-a".to_string()),
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
        let peer = PeerIdentity {
            credentials: PeerCredentials {
                pid: 202,
                uid: 0,
                gid: 0,
            },
            principal: PeerPrincipal {
                uid: 0,
                container_id: Some("container-b".to_string()),
                pid_namespace: "pid:[202]".to_string(),
                host_pid_namespace: false,
            },
        };
        let command = ControlCommand::TrackRemove(TrackRemoveCommand {
            request_id: RequestId::new(1),
            selector: TraceSelector::TraceId(trace_id),
        });

        let error = host
            .authorize_peer_command(&peer, &command, Some(trace_id))
            .unwrap_err();

        assert_eq!(error.code, "peer_identity");
        assert!(error.message.contains("not authorized"));
    }
}
