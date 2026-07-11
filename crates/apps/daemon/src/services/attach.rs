//! Attach service backed by procfs bootstrap and storage persistence.

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::RawFd;
use std::time::{Duration, SystemTime};

#[path = "attach/debug.rs"]
mod debug;
#[path = "attach/factory.rs"]
mod factory;
#[path = "attach/helpers.rs"]
mod helpers;
#[path = "attach/logging.rs"]
mod logging;
#[path = "attach/plugins.rs"]
mod plugins;
#[path = "attach/preflight.rs"]
mod preflight;

use collector_binding::TraceBindingRequest;
use collector_instance::CollectorInstance;
use config_core::capture_profile::{
    DeploymentPermissionAvailability, DeploymentPermissionPolicy, LaunchSeccompRequirements,
    PermissionMode, resolve_deployment_permissions,
};
use config_core::daemon::{
    DiagnosticLogLevel, FileObservationConfig, PayloadRedactionPolicy, PayloadStdioStorageMode,
    SemanticRetentionConfig,
};
use config_core::trace_snapshot::CaptureProfileSnapshot;
use control_contract::command::{
    DeploymentPermissionMode, ProcessRef, ResolveLaunchPermissionsCommand,
    ResolveLaunchTlsPlanCommand, TrackAddCommand,
};
use control_contract::reply::{
    ControlError, LaunchPermissionsReply, LaunchTlsPlanReply, PluginCommandReply, TrackAddReply,
};
use ebpf_collector::EbpfCollector;
use ebpf_collector::procfs::{
    ProcfsIdentityReader, ProcfsTreeSnapshotter, read_container_identity, resolve_namespaced_pid,
};
use export_core::ExportRuntime;
use model_core::capability::Capability;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::process::{ProcessIdentity, ProcessObservation, ProcessRecord};
use plugin_system::PluginInstanceStatus;
use process_identity::ProcessIdentityError;
use process_identity::ProcessIdentityManager;
use provider_label::ProviderClassifier;
use recording_runtime::{RecordingWriter, TraceStateRecord};
use semantic_action_runtime::LiveSemanticActionRuntime;
use storage_core::StorageBackend;
use trace_runtime::commands::TrackTraceRequest;
use trace_runtime::sensor_plan::SensorPlan;

use crate::profiles::DaemonProfileRegistry;
use crate::service_host::AttachService;
use crate::services::application_protocol::ApplicationProtocolAnalyzer;
use crate::services::command_control::CommandControlService;
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::enforcement::FanotifyEnforcementService;
use crate::services::network_control::NetworkControlService;
use crate::services::payload_gate::{PayloadBodyRetentionGate, SocketHttpPayloadGate};
use crate::services::process_seccomp::{ProcessSeccompObservation, ProcessSeccompService};
use crate::services::resource_metrics::ResourceMetricsSampler;
use crate::services::retention::StorageRetentionService;
use crate::services::seccomp_notify::SeccompNotifyService;
use crate::services::seccomp_socket::SeccompSocketService;
use crate::services::seccomp_tls::SeccompTlsService;
use crate::services::tls_sync::TlsSyncService;
use crate::services::workload_diagnostics::WorkloadDiagnostics;

use self::helpers::{capability_requested, collector_capability_requests};

pub(crate) struct StorageAttachService {
    pub(super) profiles: DaemonProfileRegistry,
    pub(super) launch_seccomp_requirements: LaunchSeccompRequirements,
    pub(super) storage: Box<dyn StorageBackend>,
    pub(super) process_registry: ProcessIdentityManager,
    pub(super) collector: EbpfCollector,
    pub(super) host_ebpf_preflight:
        BTreeMap<model_core::ids::ProfileName, preflight::EbpfPreflightReport>,
    pub(super) identity_reader: ProcfsIdentityReader,
    pub(super) snapshotter: ProcfsTreeSnapshotter,
    pub(super) next_event_id: u64,
    pub(super) next_diagnostic_id: u64,
    pub(super) next_payload_segment_id: u64,
    pub(super) payload_tls_enabled: bool,
    pub(super) diagnostic_log_level: DiagnosticLogLevel,
    pub(super) last_payload_tls_diagnostics: Option<String>,
    pub(super) payload_tls_redaction_policy: PayloadRedactionPolicy,
    pub(super) payload_tls_retention_max_bytes_per_trace: u64,
    pub(super) payload_stdio_enabled: bool,
    pub(super) payload_stdio_redaction_policy: PayloadRedactionPolicy,
    pub(super) payload_stdio_retention_max_bytes_per_trace: u64,
    pub(super) payload_stdio_stdin_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_stdio_stdout_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_stdio_stderr_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_socket_enabled: bool,
    pub(super) payload_socket_redaction_policy: PayloadRedactionPolicy,
    pub(super) payload_socket_retention_max_bytes_per_trace: u64,
    pub(super) socket_payload_gate: SocketHttpPayloadGate,
    pub(super) payload_body_retention_gate: PayloadBodyRetentionGate,
    pub(super) seccomp_notify: SeccompNotifyService,
    pub(super) seccomp_tls: SeccompTlsService,
    pub(super) tls_sync: TlsSyncService,
    pub(super) seccomp_socket: SeccompSocketService,
    pub(super) process_seccomp: ProcessSeccompService,
    pub(super) command_control: CommandControlService,
    pub(super) network_control: NetworkControlService,
    pub(super) pending_process_seccomp_observations: Vec<ProcessSeccompObservation>,
    pub(super) semantic_retention: SemanticRetentionConfig,
    pub(super) file_observation: FileObservationConfig,
    pub(super) application_protocol: ApplicationProtocolAnalyzer,
    pub(super) resource_metrics: ResourceMetricsSampler,
    pub(super) storage_retention: StorageRetentionService,
    pub(super) enforcement: FanotifyEnforcementService,
    pub(super) control_plugins: ControlPluginRuntime,
    pub(super) semantic_actions: LiveSemanticActionRuntime,
    pub(super) export_runtime: ExportRuntime,
    pub(super) workload_diagnostics: WorkloadDiagnostics,
    pub(super) retained_payload_bytes_by_trace: BTreeMap<model_core::ids::TraceId, u64>,
    pub(super) finalized_terminal_traces: BTreeSet<model_core::ids::TraceId>,
    pub(super) pending_terminal_finalizations: BTreeSet<model_core::ids::TraceId>,
    pub(super) finalization_traces_per_cycle: usize,
    pub(super) finalization_poll_interval: Duration,
    pub(super) diagnosed_terminal_open_memberships:
        BTreeSet<(model_core::ids::TraceId, ProcessIdentity)>,
    pub(super) provider_classifier: Box<dyn ProviderClassifier>,
    pub(super) provider_classification_enabled: bool,
}

impl StorageAttachService {
    pub(super) fn resolve_process_observation(
        &mut self,
        observation: ProcessObservation,
    ) -> Result<(ProcessIdentity, Option<ProcessRecord>), ControlError> {
        let resolution = loop {
            match self.process_registry.resolve_or_create(observation.clone()) {
                Ok(resolution) => break resolution,
                Err(ProcessIdentityError::IdBlockExhausted) => {
                    let block_size = factory::process_id_block_size()?;
                    let (block_start, block_end) = self
                        .storage
                        .reserve_process_id_block(block_size)
                        .map_err(|error| ControlError::new(error.stage, error.message))?;
                    self.process_registry
                        .install_reserved_block(block_start, block_end)
                        .map_err(|error| {
                            ControlError::new("process_id_block_install", format!("{error:?}"))
                        })?;
                }
                Err(error) => {
                    return Err(ControlError::new(
                        "process_identity_resolution",
                        format!("{error:?}"),
                    ));
                }
            }
        };
        let changed = resolution.created || resolution.enriched;
        let record = changed
            .then(|| self.process_registry.record(resolution.identity).cloned())
            .flatten();
        Ok((resolution.identity, record))
    }

    pub(crate) fn collector_name(&self) -> String {
        self.collector.descriptor().name.to_string()
    }

    pub(crate) fn collector_ready(&self) -> bool {
        self.collector.probe_result().reason_unavailable.is_none()
    }

    pub(crate) fn any_host_ebpf_preflight_available(&self) -> bool {
        self.host_ebpf_preflight
            .values()
            .any(|report| report.available)
    }

    pub(crate) fn collector_descriptor(&self) -> collector_capability::CollectorDescriptor {
        self.collector.descriptor().clone()
    }

    pub(crate) fn set_id_seeds(&mut self, next_event_id: u64, next_diagnostic_id: u64) {
        self.next_event_id = next_event_id;
        self.next_diagnostic_id = next_diagnostic_id;
    }

    pub(crate) fn set_payload_segment_id_seed(&mut self, next_payload_segment_id: u64) {
        self.next_payload_segment_id = next_payload_segment_id;
    }

    fn bootstrap_snapshot(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        profile_snapshot: CaptureProfileSnapshot,
        sensor_plan: SensorPlan,
    ) -> Result<BootstrapSnapshot, ControlError> {
        let root_observation = resolve_process_ref(&command.root)?;
        let root_host_pid = root_observation
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| ControlError::new("pid_resolution", "root host PID is missing"))?;
        let (root_identity, root_record) =
            self.resolve_process_observation(root_observation.clone())?;
        // Resolve the container id host-side from the already-resolved host pid.
        // This sees the full `docker-<id>` cgroup path; a container-local
        // `/proc/self/cgroup` may be masked to `0::/`). `None` = host/non-Docker.
        let root_container_id =
            read_container_identity(root_host_pid).map(|identity| identity.container_id);
        let snapshot = process_tree_snapshot_contract::snapshot::ProcessTreeSnapshotter::snapshot(
            &self.snapshotter,
            &root_observation,
        )
        .map_err(|error| ControlError::new("snapshot", error))?;

        let trace_id = trace_runtime.reserve_trace_id();
        trace_runtime
            .create_starting_trace(
                trace_id,
                TrackTraceRequest {
                    root_identity: root_identity.clone(),
                    root_container_id,
                    display_name: command.display_name.clone(),
                    profile_snapshot,
                    tags: command.tags.clone(),
                    created_at: SystemTime::now(),
                },
                sensor_plan,
            )
            .map_err(|error| ControlError::new("create_trace", format!("{:?}", error)))?;

        let mut process_records = BTreeMap::new();
        if let Some(record) = root_record {
            process_records.insert(record.identity, record);
        }
        let mut bootstrap_partial = false;
        for process in snapshot.processes {
            let (identity, record) = self.resolve_process_observation(process.identity)?;
            if let Some(record) = record {
                process_records.insert(record.identity, record);
            }
            if identity == root_identity {
                continue;
            }
            let Some(parent_observation) = process.parent else {
                bootstrap_partial = true;
                continue;
            };
            let (parent, parent_record) = self.resolve_process_observation(parent_observation)?;
            if let Some(record) = parent_record {
                process_records.insert(record.identity, record);
            }
            let membership = model_core::process::ProcessMembership::inherited(
                trace_id,
                identity,
                parent,
                snapshot.captured_at,
            );
            trace_runtime
                .insert_membership(trace_id, membership)
                .map_err(|error| ControlError::new("insert_membership", format!("{:?}", error)))?;
        }
        Ok(BootstrapSnapshot {
            trace_id,
            root_identity,
            root_observation,
            process_records: process_records.into_values().collect(),
            diagnostic_kind: if bootstrap_partial {
                DiagnosticKind::BootstrapPartial
            } else {
                DiagnosticKind::BootstrapGap
            },
        })
    }

    fn finalize_trace(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        trace_id: model_core::ids::TraceId,
        root_identity: ProcessIdentity,
        process_records: Vec<ProcessRecord>,
        launch_mode: bool,
        diagnostic_kind: DiagnosticKind,
        diagnostic_message: &'static str,
    ) -> Result<TrackAddReply, ControlError> {
        let emit_bootstrap_diagnostic =
            !launch_mode || matches!(diagnostic_kind, DiagnosticKind::BootstrapPartial);
        if emit_bootstrap_diagnostic {
            trace_runtime
                .mark_degraded(trace_id)
                .map_err(|error| ControlError::new("mark_degraded", format!("{:?}", error)))?;
        }
        trace_runtime
            .activate_trace(trace_id, SystemTime::now())
            .map_err(|error| ControlError::new("activate_trace", format!("{:?}", error)))?;

        let entry = trace_runtime.get_trace(trace_id).ok_or_else(|| {
            ControlError::new("trace_missing", "trace disappeared after activation")
        })?;
        let trace = entry.trace.clone();
        let trace_state = TraceStateRecord::new(
            trace.clone(),
            entry.memberships.memberships().cloned().collect(),
        );
        RecordingWriter::new(self.storage.as_mut())
            .persist_trace_state_with_process_records(trace_state, process_records)
            .map_err(recording_error_to_control)?;
        if emit_bootstrap_diagnostic {
            let diagnostic = DiagnosticRecord::new(
                self.next_diagnostic_id()?,
                Some(trace_id),
                diagnostic_kind,
                DiagnosticSeverity::Warning,
                SystemTime::now(),
                diagnostic_message,
            )
            .with_process(root_identity);
            RecordingWriter::new(self.storage.as_mut())
                .persist_diagnostic(diagnostic)
                .map_err(recording_error_to_control)?;
        }
        if launch_mode {
            self.log_diagnostic(
                DiagnosticLogLevel::Info,
                format_args!(
                    "agent_launch started trace_id={} name={} process_id={}",
                    trace_id,
                    trace.display_name,
                    trace.root_process_identity.get()
                ),
            );
        }

        Ok(TrackAddReply {
            trace_id,
            lifecycle_state: trace.lifecycle_state,
        })
    }

    fn attach_snapshot_only(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        profile_snapshot: CaptureProfileSnapshot,
        sensor_plan: SensorPlan,
    ) -> Result<TrackAddReply, ControlError> {
        let bootstrap =
            self.bootstrap_snapshot(trace_runtime, command, profile_snapshot, sensor_plan)?;
        self.finalize_trace(
            trace_runtime,
            bootstrap.trace_id,
            bootstrap.root_identity,
            bootstrap.process_records,
            command.launch_mode,
            bootstrap.diagnostic_kind,
            "snapshot-only attach completed without eBPF coverage guard",
        )
    }

    fn attach_with_collector(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        profile_snapshot: CaptureProfileSnapshot,
        sensor_plan: SensorPlan,
    ) -> Result<TrackAddReply, ControlError> {
        let collector_name = self.collector_name();
        let requested_capabilities = collector_capability_requests(
            &profile_snapshot.capability_requests,
            &sensor_plan,
            &collector_name,
        );
        let uses_ebpf_collector = !requested_capabilities.is_empty();
        let bootstrap = self.bootstrap_snapshot(
            trace_runtime,
            command,
            profile_snapshot.clone(),
            sensor_plan,
        )?;

        let member_processes = trace_runtime
            .get_trace(bootstrap.trace_id)
            .ok_or_else(|| {
                ControlError::new("trace_missing", "trace disappeared during bootstrap")
            })?
            .memberships
            .memberships()
            .map(|membership| {
                self.process_registry
                    .record(membership.identity)
                    .cloned()
                    .ok_or_else(|| {
                        ControlError::new(
                            "process_registry",
                            format!("missing process record {}", membership.identity.get()),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        if uses_ebpf_collector {
            if let Err(error) = self.collector.bind_trace(&TraceBindingRequest {
                trace_id: bootstrap.trace_id,
                root_identity: bootstrap.root_identity,
                root_observation: bootstrap.root_observation.clone(),
                root_namespace_pid: command.root.namespace_pid,
                profile_snapshot: profile_snapshot.clone(),
                requested_capabilities,
                initial_suppressed_fds: command.initial_suppressed_fds.clone(),
            }) {
                let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                return Err(ControlError::new(error.stage, error.message));
            }

            if let Err(error) = self
                .collector
                .seed_trace_memberships(bootstrap.trace_id, member_processes)
            {
                let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                return Err(ControlError::new(error.stage, error.message));
            }
        }

        self.finalize_trace(
            trace_runtime,
            bootstrap.trace_id,
            bootstrap.root_identity,
            bootstrap.process_records,
            command.launch_mode,
            bootstrap.diagnostic_kind,
            if uses_ebpf_collector {
                "snapshot bootstrap completed before live eBPF tracking and remains gap-marked"
            } else {
                "snapshot bootstrap completed before virtual collector sampling and remains gap-marked"
            },
        )
    }
}

impl AttachService for StorageAttachService {
    fn host_pid_for_process(&self, process: ProcessIdentity) -> Result<u32, ControlError> {
        self.process_registry
            .record(process)
            .and_then(|record| record.host.as_ref())
            .map(|host| host.pid)
            .ok_or_else(|| {
                ControlError::new(
                    "process_host_pid",
                    format!("process {} has no host PID", process.get()),
                )
            })
    }

    fn resolve_launch_permissions(
        &mut self,
        command: &ResolveLaunchPermissionsCommand,
        host_ebpf_available: bool,
    ) -> Result<LaunchPermissionsReply, ControlError> {
        let profile = self
            .profiles
            .capture_profile(&command.profile_name)
            .ok_or_else(|| {
                ControlError::new("unknown_profile", "capture profile does not exist")
            })?;
        let policy = DeploymentPermissionPolicy {
            host_ebpf: permission_mode(command.host_ebpf),
            seccomp_notify: permission_mode(command.seccomp_notify),
        };
        let decision = resolve_deployment_permissions(
            policy,
            profile,
            self.launch_seccomp_requirements,
            &DeploymentPermissionAvailability {
                host_ebpf: Some(host_ebpf_available),
                seccomp_notify: Some(command.seccomp_notify_available),
                seccomp_notify_detail: command.seccomp_notify_detail.clone(),
            },
        )
        .map_err(|message| ControlError::new("deployment_permissions", message))?;
        let selected_profile = decision.selected_profile(profile);
        if self
            .profiles
            .capture_profile(&selected_profile.name)
            .is_none()
        {
            return Err(ControlError::new(
                "unknown_profile",
                "daemon did not register the selected deployment profile",
            ));
        }
        let effective_seccomp = self
            .launch_seccomp_requirements
            .enabled_by(decision.selected.seccomp_notify);
        Ok(LaunchPermissionsReply {
            requested_host_ebpf: contract_permission_mode(decision.requested_host_ebpf),
            requested_seccomp_notify: contract_permission_mode(decision.requested_seccomp_notify),
            selected_host_ebpf: decision.selected.host_ebpf,
            selected_seccomp_notify: decision.selected.seccomp_notify,
            selected_profile_name: selected_profile.name,
            payload_tls_seccomp: effective_seccomp.payload_tls,
            payload_socket_seccomp: effective_seccomp.payload_socket,
            process_seccomp: effective_seccomp.process_seccomp,
            network_control_seccomp: effective_seccomp.network_control,
            required_capabilities: decision.required_capabilities,
            degraded: decision.degraded,
            reasons: decision.reasons,
        })
    }

    fn host_ebpf_available_for_profile(&self, profile_name: &model_core::ids::ProfileName) -> bool {
        self.host_ebpf_preflight_available_for_profile(profile_name)
    }

    fn resolve_launch_tls_plan(
        &mut self,
        command: &ResolveLaunchTlsPlanCommand,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        self.tls_sync.resolve_launch_plan(&command.binary)
    }

    fn attach_existing(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
    ) -> Result<TrackAddReply, ControlError> {
        if !command.launch_mode && self.profiles.is_launch_only_profile(&command.profile_name) {
            return Err(ControlError::new(
                "launch_admission",
                "deployment-derived profiles require a daemon launch permission admission",
            ));
        }
        let profile = self
            .profiles
            .capture_profile(&command.profile_name)
            .ok_or_else(|| {
                ControlError::new("unknown_profile", "capture profile does not exist")
            })?;
        let profile_snapshot = CaptureProfileSnapshot::from_profile(profile, SystemTime::now());
        if self.seccomp_tls.enabled()
            && !command.launch_mode
            && capability_requested(
                &profile_snapshot.capability_requests,
                &Capability::TlsPlaintextPayload,
            )
        {
            return Err(ControlError::new(
                "payload_tls_backend",
                "TLS plaintext payload capture is only supported by actrailctl launch",
            ));
        }
        if self.process_seccomp.enabled()
            && !command.launch_mode
            && capability_requested(
                &profile_snapshot.capability_requests,
                &Capability::ProcExecContext,
            )
        {
            return Err(ControlError::new(
                "process_seccomp_backend",
                "process exec context capture is only supported by actrailctl launch",
            ));
        }
        let sensor_plan = trace_runtime
            .negotiate(&profile_snapshot)
            .map_err(|error| ControlError::new("negotiate", format!("{:?}", error)))?;

        if sensor_plan.collectors.is_empty() {
            return self.attach_snapshot_only(
                trace_runtime,
                command,
                profile_snapshot,
                sensor_plan,
            );
        }

        self.attach_with_collector(trace_runtime, command, profile_snapshot, sensor_plan)
    }

    fn drain_live_events(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        self.drain_live_events_impl(trace_runtime)
    }

    fn event_poll_fds(&self) -> Result<Vec<RawFd>, ControlError> {
        let mut fds = Vec::new();
        if let Some(fd) = self
            .collector
            .event_poll_fd()
            .map_err(|error| ControlError::new(error.stage, error.message))?
        {
            fds.push(fd);
        }
        fds.extend(self.enforcement.event_poll_fds());
        fds.extend(self.tls_sync.event_poll_fds());
        fds.extend(self.seccomp_notify.event_poll_fds());
        Ok(fds)
    }

    fn background_poll_timeout(&self) -> Result<Option<Duration>, ControlError> {
        let mut timeout = self.resource_metrics.poll_timeout();
        timeout = min_optional_timeout(
            timeout,
            (!self.pending_terminal_finalizations.is_empty())
                .then_some(self.finalization_poll_interval),
        );
        timeout = min_optional_timeout(timeout, self.storage_retention.poll_timeout());
        Ok(timeout)
    }

    fn remove_root(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        trace_id: model_core::ids::TraceId,
        removed_at: SystemTime,
    ) -> Result<(), ControlError> {
        self.remove_root_impl(trace_runtime, trace_id, removed_at)
    }

    fn register_seccomp_listener(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: control_contract::command::RegisterSeccompListenerCommand,
    ) -> Result<(), ControlError> {
        let target_observation = resolve_process_ref(&command.target)?;
        let target_pid = target_observation
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| ControlError::new("seccomp_listener", "target host PID is missing"))?;
        let (target_identity, target_record) =
            self.resolve_process_observation(target_observation)?;
        let trace = trace_runtime
            .get_trace(command.trace_id)
            .ok_or_else(|| ControlError::new("seccomp_listener", "trace not found"))?;
        let target_known = trace
            .memberships
            .memberships()
            .any(|membership| membership.identity == target_identity);
        if !target_known {
            let inherited = self.process_seccomp.ensure_listener_target(
                trace_runtime,
                &self.process_registry,
                &self.identity_reader,
                command.trace_id,
                target_pid,
            )?;
            if let Some(identity) = inherited {
                let record = target_record
                    .or_else(|| self.process_registry.record(identity).cloned())
                    .ok_or_else(|| {
                        ControlError::new("process_registry", "listener process record is missing")
                    })?;
                if self.collector.stats().active_bindings > 0 {
                    self.collector
                        .seed_trace_memberships(command.trace_id, std::iter::once(record.clone()))
                        .map_err(|error| ControlError::new(error.stage, error.message))?;
                }
                self.storage
                    .upsert_process_record(record)
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
                self.persist_trace_state(trace_runtime, command.trace_id)?;
            }
        }
        self.seccomp_notify.register_listener(command.listener_fd)
    }

    fn plugin_statuses(&self) -> Vec<PluginInstanceStatus> {
        self.plugin_statuses_impl()
    }

    fn load_plugin(
        &mut self,
        command: control_contract::command::PluginLoadCommand,
    ) -> Result<PluginInstanceStatus, ControlError> {
        self.load_plugin_impl(command)
    }

    fn unload_plugin(&mut self, instance_id: &str) -> Result<PluginInstanceStatus, ControlError> {
        self.unload_plugin_impl(instance_id)
    }

    fn handle_plugin_command(
        &mut self,
        command: control_contract::command::PluginCommandCommand,
    ) -> Result<PluginCommandReply, ControlError> {
        if command.instance_id.trim().is_empty() {
            return Err(ControlError::new(
                "plugin_command",
                "plugin instance id must not be empty",
            ));
        }
        if command.argv.is_empty() {
            return Err(ControlError::new(
                "plugin_command",
                "plugin command argv must not be empty",
            ));
        }
        let response = self
            .control_plugins
            .handle_command(
                &command.instance_id,
                plugin_system::PluginCommandRequest { argv: command.argv },
                plugin_system::PluginCommandBudget::default(),
            )
            .map_err(|error| ControlError::new(error.code, error.message))?;
        Ok(PluginCommandReply {
            instance_id: command.instance_id,
            exit_code: response.exit_code,
            stdout: response.stdout,
            stderr: response.stderr,
        })
    }
}

fn permission_mode(mode: DeploymentPermissionMode) -> PermissionMode {
    match mode {
        DeploymentPermissionMode::Auto => PermissionMode::Auto,
        DeploymentPermissionMode::Required => PermissionMode::Required,
        DeploymentPermissionMode::Disabled => PermissionMode::Disabled,
    }
}

fn contract_permission_mode(mode: PermissionMode) -> DeploymentPermissionMode {
    match mode {
        PermissionMode::Auto => DeploymentPermissionMode::Auto,
        PermissionMode::Required => DeploymentPermissionMode::Required,
        PermissionMode::Disabled => DeploymentPermissionMode::Disabled,
    }
}

fn recording_error_to_control(error: recording_runtime::RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}

fn resolve_process_ref(process: &ProcessRef) -> Result<ProcessObservation, ControlError> {
    resolve_namespaced_pid(process.namespace_pid, &process.pid_namespace)
        .map_err(|error| ControlError::new("pid_resolution", error))
}

struct BootstrapSnapshot {
    trace_id: model_core::ids::TraceId,
    root_identity: ProcessIdentity,
    root_observation: ProcessObservation,
    process_records: Vec<ProcessRecord>,
    diagnostic_kind: DiagnosticKind,
}

fn min_optional_timeout(left: Option<Duration>, right: Option<Duration>) -> Option<Duration> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(timeout), None) | (None, Some(timeout)) => Some(timeout),
        (None, None) => None,
    }
}
