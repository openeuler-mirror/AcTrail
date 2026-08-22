//! Host boundary for long-lived daemon services.

use std::collections::BTreeMap;
use std::os::fd::{OwnedFd, RawFd};
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
use sandbox_evidence_store::SandboxEvidenceWritePort;
use uds_control_server::{ControlService, PeerCredentials};

use crate::peer_identity::{PeerIdentity, peer_error};
use crate::runtime_wiring::DaemonRuntimeWiring;
use crate::services::sandbox_plugins::{SandboxPluginManager, SandboxPluginRouteSink};

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
    fn attach_launch(
        &mut self,
        _trace_runtime: &mut trace_runtime::TraceRuntime,
        _command: &control_contract::command::TrackAddCommand,
        _pidfd: OwnedFd,
    ) -> Result<TrackAddReply, ControlError> {
        Err(ControlError::new(
            "launch_pidfd",
            "attach service does not implement pidfd launch registration",
        ))
    }
    fn drain_live_events(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError>;
    fn event_poll_fds(&self) -> Result<Vec<RawFd>, ControlError>;
    fn background_poll_timeout(&self) -> Result<Option<Duration>, ControlError>;
    fn shutdown(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError>;
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
    sandbox_plugins: SandboxPluginManager,
    pending_launch_admissions: BTreeMap<model_core::process::ProcessObservation, LaunchAdmission>,
}

impl<A> DaemonServiceHost<A> {
    pub fn new(wiring: DaemonRuntimeWiring<A>) -> Self {
        Self {
            wiring,
            sandbox_plugins: SandboxPluginManager::new(),
            pending_launch_admissions: BTreeMap::new(),
        }
    }

    pub(crate) fn sandbox_route_sink(
        &self,
        archive: std::sync::Arc<dyn SandboxEvidenceWritePort>,
    ) -> SandboxPluginRouteSink {
        self.sandbox_plugins.route_sink(archive)
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
        let sandbox_result = self.sandbox_plugins.shutdown();
        let attach_result = self
            .wiring
            .attach_service
            .shutdown(&mut self.wiring.trace_runtime);
        sandbox_result.and(attach_result)
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
        if self
            .plugin_statuses()
            .iter()
            .any(|status| status.instance_id == command.instance_id)
        {
            return Err(ControlError::new(
                "plugin_runtime",
                format!("plugin instance {} already exists", command.instance_id),
            ));
        }
        let manifest_path = std::path::Path::new(&command.manifest_path);
        if SandboxPluginManager::is_sandbox_manifest(manifest_path)? {
            self.sandbox_plugins.load(command)
        } else {
            self.wiring.attach_service.load_plugin(command)
        }
    }

    fn plugin_statuses(&self) -> Vec<PluginInstanceStatus>
    where
        A: AttachService,
    {
        let mut statuses = self.wiring.attach_service.plugin_statuses();
        statuses.extend(self.sandbox_plugins.statuses());
        statuses
    }

    fn plugin_status(&self, instance_id: &str) -> Result<PluginInstanceStatus, ControlError>
    where
        A: AttachService,
    {
        self.plugin_statuses()
            .into_iter()
            .find(|status| status.instance_id == instance_id)
            .ok_or_else(|| {
                ControlError::new(
                    "plugin_not_found",
                    format!("plugin instance {instance_id} not found"),
                )
            })
    }

    fn unload_plugin(&mut self, instance_id: &str) -> Result<PluginInstanceStatus, ControlError>
    where
        A: AttachService,
    {
        if self.sandbox_plugins.contains(instance_id) {
            self.sandbox_plugins.unload(instance_id)
        } else {
            self.wiring.attach_service.unload_plugin(instance_id)
        }
    }

    fn sandbox_plugin_capability_error(instance_id: &str) -> ControlError {
        ControlError::new(
            "plugin_capability",
            format!(
                "sandbox plugin instance {instance_id} is configured by its package config and must be reloaded to change it"
            ),
        )
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
        self.handle_from_peer_with_launch_pidfd(credentials, command, None)
    }

    fn handle_from_peer_with_launch_pidfd(
        &mut self,
        credentials: PeerCredentials,
        command: ControlCommand,
        launch_pidfd: Option<OwnedFd>,
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

        let mut reply = self.handle_with_launch_pidfd(command, launch_pidfd)?;
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
        self.handle_with_launch_pidfd(command, None)
    }

    fn handle_with_launch_pidfd(
        &mut self,
        command: ControlCommand,
        mut launch_pidfd: Option<OwnedFd>,
    ) -> Result<ControlReply, ControlError> {
        if launch_pidfd.is_some()
            && !matches!(
                &command,
                ControlCommand::TrackAdd(track_add) if track_add.launch_mode
            )
        {
            return Err(ControlError::new(
                "unexpected_fd",
                "launch pidfd is only valid for launch-mode track-add",
            ));
        }
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
                if command.launch_mode {
                    let pidfd = launch_pidfd.take().ok_or_else(|| {
                        ControlError::new(
                            "launch_pidfd",
                            "launch-mode track-add requires an SCM_RIGHTS pidfd",
                        )
                    })?;
                    self.wiring
                        .attach_service
                        .attach_launch(&mut self.wiring.trace_runtime, &command, pidfd)
                        .map(ControlReply::TrackAdded)
                } else {
                    self.wiring
                        .attach_service
                        .attach_existing(&mut self.wiring.trace_runtime, &command)
                        .map(ControlReply::TrackAdded)
                }
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
                            root_pid_namespace: trace.root_pid_namespace.clone(),
                            root_container_id: trace.root_container_id.clone(),
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
            ControlCommand::PluginList(_) => Ok(ControlReply::PluginList(self.plugin_statuses())),
            ControlCommand::PluginStatus(command) => self
                .plugin_status(&command.instance_id)
                .map(ControlReply::PluginStatus),
            ControlCommand::PluginLoad(command) => {
                self.load_plugin(command).map(ControlReply::PluginStatus)
            }
            ControlCommand::PluginUnload(command) => self
                .unload_plugin(&command.instance_id)
                .map(ControlReply::PluginStatus),
            ControlCommand::PluginCommand(command) => {
                if self.sandbox_plugins.contains(&command.instance_id) {
                    Err(Self::sandbox_plugin_capability_error(&command.instance_id))
                } else {
                    self.wiring
                        .attach_service
                        .handle_plugin_command(command)
                        .map(ControlReply::PluginCommand)
                }
            }
            ControlCommand::PluginConfigGet(command) => {
                if self.sandbox_plugins.contains(&command.instance_id) {
                    Err(Self::sandbox_plugin_capability_error(&command.instance_id))
                } else {
                    self.wiring
                        .attach_service
                        .plugin_config(&command.instance_id)
                        .map(ControlReply::PluginConfig)
                }
            }
            ControlCommand::PluginConfigValidate(command) => {
                if self.sandbox_plugins.contains(&command.instance_id) {
                    Err(Self::sandbox_plugin_capability_error(&command.instance_id))
                } else {
                    self.wiring
                        .attach_service
                        .validate_plugin_config(&command.instance_id, &command.config_json)
                        .map(ControlReply::PluginConfigValidation)
                }
            }
            ControlCommand::PluginConfigUpdate(command) => {
                if self.sandbox_plugins.contains(&command.instance_id) {
                    Err(Self::sandbox_plugin_capability_error(&command.instance_id))
                } else {
                    self.wiring
                        .attach_service
                        .update_plugin_config(&command.instance_id, &command.config_json)
                        .map(ControlReply::PluginConfig)
                }
            }
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
